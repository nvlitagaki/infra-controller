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
use axum::Router;
use axum::body::Body;
use carbide_uuid::machine::MachineType;
use http_body_util::BodyExt;
use hyper::http::StatusCode;
use tower::ServiceExt;

use super::env::TestEnv;
use super::{make_test_app, web_request_builder};

async fn get_body(app: &Router, uri: &str) -> String {
    let response = app
        .clone()
        .oneshot(web_request_builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{uri}");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(body.to_vec()).unwrap()
}

fn html_dpu_ids(html: &str) -> Vec<String> {
    html.split("/admin/machine/")
        .skip(1)
        .filter_map(|part| part.split('"').next())
        .filter(|id| id.starts_with(MachineType::Dpu.id_prefix()))
        .map(str::to_owned)
        .collect()
}

async fn json_dpu_ids(app: &Router) -> Vec<String> {
    let body = get_body(app, "/admin/dpu/versions.json").await;
    let rows: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    rows.iter()
        .map(|row| row["dpu_machine_id"].as_str().unwrap().to_owned())
        .collect()
}

#[crate::sqlx_test]
async fn dpu_versions_backend_pages_match_json(pool: sqlx::PgPool) {
    let env = TestEnv::new(pool).await;
    let app = make_test_app(&env.test_harness);
    assert!(json_dpu_ids(&app).await.is_empty());
    assert!(html_dpu_ids(&get_body(&app, "/admin/dpu/versions?limit=25").await).is_empty());
    let mut expected = Vec::new();
    // 104 DPUs cross the 100-record All/JSON batch boundary. Four DPUs per host
    // keep address use within the fixture's existing /24 underlay pool.
    for _ in 0..26 {
        let (host, _) = env.create_ready_managed_host(4).await;
        expected.extend(host.dpus.iter().map(|dpu| dpu.id.to_string()));
    }
    expected.sort_unstable();
    assert_eq!(expected.len(), 104);
    for _ in 0..2 {
        assert_eq!(json_dpu_ids(&app).await, expected);
        let mut actual = Vec::new();
        for (page, expected_page) in expected.chunks(25).enumerate() {
            let html = get_body(
                &app,
                &format!("/admin/dpu/versions?limit=25&current_page={page}"),
            )
            .await;
            assert!(html.contains("DPU Versions (104)"));
            let ids = html_dpu_ids(&html);
            assert_eq!(ids, expected_page);
            actual.extend(ids);
        }
        assert_eq!(actual, expected);
        assert_eq!(json_dpu_ids(&app).await, expected);
    }

    let empty_page = expected.len().div_ceil(25);
    let html = get_body(
        &app,
        &format!("/admin/dpu/versions?limit=25&current_page={empty_page}"),
    )
    .await;
    assert!(html_dpu_ids(&html).is_empty());

    let html = get_body(&app, "/admin/dpu/versions?limit=0").await;
    assert_eq!(html_dpu_ids(&html), expected);
}
