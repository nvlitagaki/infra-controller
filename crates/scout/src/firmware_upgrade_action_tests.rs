/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Response, Uri};
use axum::routing::get;
use clap::Parser;
use sha2::{Digest, Sha256};

use super::*;

#[derive(Default)]
struct Calls {
    accepted: bool,
    acknowledgements: usize,
    reports: usize,
    lose_ack_once: bool,
    lose_report_once: bool,
}

async fn grpc_stub(State(calls): State<Arc<Mutex<Calls>>>, uri: Uri) -> Response<Body> {
    let mut calls = calls.lock().unwrap();
    let status = match uri.path() {
        "/forge.Forge/Version" => None,
        "/forge.Forge/AcknowledgeScoutFirmwareUpgrade" => {
            calls.acknowledgements += 1;
            calls.accepted = true;
            if std::mem::take(&mut calls.lose_ack_once) {
                Some(tonic::Status::unavailable("injected lost ack response"))
            } else {
                None
            }
        }
        "/forge.Forge/ReportScoutFirmwareUpgradeStatus" => {
            calls.reports += 1;
            if !calls.accepted {
                Some(tonic::Status::failed_precondition(
                    "Scout firmware task has not been acknowledged",
                ))
            } else if std::mem::take(&mut calls.lose_report_once) {
                Some(tonic::Status::unavailable("injected lost report response"))
            } else {
                None
            }
        }
        other => panic!("unexpected RPC {other}"),
    };
    match status {
        Some(status) => status.into_http::<Body>(),
        None => Response::builder()
            .header("content-type", "application/grpc")
            .header("grpc-status", "0")
            .body(Body::from(vec![0_u8; 5]))
            .unwrap(),
    }
}

async fn setup(
    calls: Arc<Mutex<Calls>>,
) -> (
    Options,
    rpc::scout_firmware_upgrade::ScoutFirmwareUpgradeTask,
    tempfile::TempDir,
    std::path::PathBuf,
) {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("executions");
    let script = format!("#!/bin/sh\nprintf run >> '{}'\n", marker.display());
    let digest = hex::encode(Sha256::digest(script.as_bytes()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = Router::new()
        .route(
            "/upgrade.sh",
            get(move || {
                let script = script.clone();
                async move { script }
            }),
        )
        .fallback(grpc_stub)
        .with_state(calls);
    tokio::spawn(async move {
        axum::serve(listener, server).await.unwrap();
    });
    let options = Options::parse_from([
        "forge-scout",
        &format!("--api={base}"),
        concat!(
            "--root-ca=",
            env!("CARGO_MANIFEST_DIR"),
            "/../../dev/certs/localhost/ca.crt"
        ),
        concat!(
            "--client-cert=",
            env!("CARGO_MANIFEST_DIR"),
            "/../../dev/certs/localhost/client.crt"
        ),
        concat!(
            "--client-key=",
            env!("CARGO_MANIFEST_DIR"),
            "/../../dev/certs/localhost/client.key"
        ),
    ]);
    let task = rpc::scout_firmware_upgrade::ScoutFirmwareUpgradeTask {
        upgrade_task_id: "firmware-retry-task".into(),
        script: Some(rpc::scout_firmware_upgrade::FileArtifact {
            url: format!("{base}/upgrade.sh"),
            sha256: digest,
        }),
        execution_timeout_seconds: 5,
        ..Default::default()
    };
    (options, task, directory, marker)
}

fn check_status(result: Result<(), CarbideClientError>, code: tonic::Code) {
    match result {
        Err(CarbideClientError::TonicStatusError(status)) => assert_eq!(status.code(), code),
        other => panic!("expected {code:?}, got {other:?}"),
    }
}

/// Run the production action handler and RPC client against loopback stubs.
/// The subprocess confines the plaintext test override to this one test; other
/// tests retain their normal TLS environment even when libtest runs in parallel.
#[test]
fn firmware_action_recovers_lost_responses_and_legacy_completion() {
    const CHILD: &str = "SCOUT_FIRMWARE_RETRY_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "firmware_upgrade_action_tests::firmware_action_recovers_lost_responses_and_legacy_completion",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("DISABLE_TLS_ENFORCEMENT", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }

    tokio::runtime::Runtime::new().unwrap().block_on(async {
        tokio::time::timeout(Duration::from_secs(30), async {
            for initial_requires_acknowledgement in [true, false] {
                let calls = Arc::new(Mutex::new(Calls {
                    lose_ack_once: true,
                    lose_report_once: true,
                    ..Default::default()
                }));
                let (config, task, _directory, marker) = setup(calls.clone()).await;
                let machine = "fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg"
                    .parse()
                    .unwrap();
                let mut state = firmware_upgrade::FirmwareUpgradeState::default();
                let action = fac::FirmwareUpgrade {
                    task: Some(task.clone()),
                    requires_acknowledgement: true,
                };
                if !initial_requires_acknowledgement {
                    // An old Core response omitted the flag. The new Core report
                    // endpoint refuses completion until acceptance is reconciled.
                    check_status(
                        handle_firmware_upgrade_action(
                            &config,
                            &machine,
                            fac::FirmwareUpgrade {
                                requires_acknowledgement: false,
                                ..action.clone()
                            },
                            &mut state,
                        )
                        .await,
                        tonic::Code::FailedPrecondition,
                    );
                    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "run");
                    assert_eq!(calls.lock().unwrap().acknowledgements, 0);
                }

                check_status(
                    handle_firmware_upgrade_action(&config, &machine, action.clone(), &mut state)
                        .await,
                    tonic::Code::Unavailable,
                );
                assert_eq!(marker.exists(), !initial_requires_acknowledgement);
                assert_eq!(
                    calls.lock().unwrap().reports,
                    usize::from(!initial_requires_acknowledgement)
                );

                // A failed acknowledgement must preserve any cached legacy result.
                // On a fresh task, execution starts only after the successful retry.
                check_status(
                    handle_firmware_upgrade_action(&config, &machine, action.clone(), &mut state)
                        .await,
                    tonic::Code::Unavailable,
                );
                assert_eq!(std::fs::read_to_string(&marker).unwrap(), "run");
                handle_firmware_upgrade_action(&config, &machine, action.clone(), &mut state)
                    .await
                    .unwrap();
                assert_eq!(std::fs::read_to_string(&marker).unwrap(), "run");
                {
                    let calls = calls.lock().unwrap();
                    assert!(calls.accepted);
                    assert_eq!(calls.acknowledgements, 2);
                    assert_eq!(
                        calls.reports,
                        2 + usize::from(!initial_requires_acknowledgement)
                    );
                }

                // A later legacy response must not forget an acceptance already
                // confirmed, even if the following response requires it again.
                for requires_acknowledgement in [false, true] {
                    handle_firmware_upgrade_action(
                        &config,
                        &machine,
                        fac::FirmwareUpgrade {
                            requires_acknowledgement,
                            ..action.clone()
                        },
                        &mut state,
                    )
                    .await
                    .unwrap();
                }
                assert_eq!(calls.lock().unwrap().acknowledgements, 2);
                assert_eq!(std::fs::read_to_string(marker).unwrap(), "run");
            }
        })
        .await
        .expect("firmware action recovery must finish within 30 seconds");
    });
}
