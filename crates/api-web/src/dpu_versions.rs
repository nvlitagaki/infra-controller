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

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::Json;
use axum::extract::{Query, State as AxumState};
use axum::response::{Html, IntoResponse};
use carbide_api_core::Api;
use hyper::http::StatusCode;
use rpc::forge as forgerpc;

use super::pagination::{self, PageContext, PaginationParams};
use super::{Base, filters};

#[derive(Template)]
#[template(path = "dpu_versions.html")]
struct DpuVersions {
    machines: Vec<Row>,
    page: PageContext,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
struct Row {
    host_machine_id: String,
    dpu_machine_id: String,
    state: String,
    dpu_type: String,
    dpu_agent_version: String,
    firmware_version: String,
    bmc_version: String,
    bios_version: String,
    hbn_version: String,
}

impl From<forgerpc::Machine> for Row {
    fn from(machine: forgerpc::Machine) -> Self {
        let state = match machine.state.split_once(' ') {
            Some((state, _)) => state.to_owned(),
            None => machine.state,
        };
        let status = machine.status.unwrap_or_default();

        Row {
            dpu_machine_id: machine.id.map(|id| id.to_string()).unwrap_or_default(),
            host_machine_id: status
                .associated_host_machine_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
            dpu_type: status
                .discovery_info
                .as_ref()
                .and_then(|di| di.dmi_data.as_ref())
                .map(|dmi_data| dmi_data.product_name.clone())
                .unwrap_or_default(),
            state,
            dpu_agent_version: machine
                .inventory
                .as_ref()
                .and_then(|inventory| {
                    inventory
                        .components
                        .iter()
                        .find(|c| c.name == "carbide-dpu-agent" || c.name == "forge-dpu-agent")
                        .map(|c| c.version.clone())
                })
                .unwrap_or_default(),
            firmware_version: status
                .discovery_info
                .as_ref()
                .and_then(|di| di.dpu_info.as_ref())
                .map(|dpu| dpu.firmware_version.clone())
                .unwrap_or_default(),
            bmc_version: machine
                .bmc_info
                .as_ref()
                .and_then(|bmc| bmc.firmware_version.clone())
                .unwrap_or_default(),
            bios_version: status
                .discovery_info
                .as_ref()
                .and_then(|di| di.dmi_data.as_ref())
                .map(|dmi_data| dmi_data.bios_version.clone())
                .unwrap_or_default(),
            hbn_version: machine
                .inventory
                .and_then(|inv| {
                    inv.components
                        .iter()
                        .find(|c| c.name.contains("hbn"))
                        .map(|c| c.version.clone())
                })
                .unwrap_or_default(),
        }
    }
}

async fn fetch_dpu_page(
    api: &Arc<Api>,
    offset: u64,
    limit: u32,
) -> Result<(Vec<forgerpc::Machine>, u64), tonic::Status> {
    use rpc::forge::forge_server::Forge;

    let page = api
        .find_machine_ids_page(tonic::Request::new(forgerpc::FindMachineIdsPageRequest {
            search_config: Some(forgerpc::MachineSearchConfig {
                include_dpus: true,
                exclude_hosts: true,
                ..Default::default()
            }),
            offset,
            limit,
        }))
        .await?
        .into_inner();
    let mut by_id = HashMap::new();
    // Respect the existing detail endpoint's configured request limit.
    let chunk_size = api.runtime_config.max_find_by_ids.max(1) as usize;
    for ids in page.machine_ids.chunks(chunk_size) {
        let response = api
            .find_machines_by_ids(tonic::Request::new(forgerpc::MachinesByIdsRequest {
                machine_ids: ids.to_vec(),
                include_history: false,
            }))
            .await?
            .into_inner();
        by_id.extend(
            response
                .machines
                .into_iter()
                .map(|machine| (machine.id, machine)),
        );
    }
    // Details arrive in HashMap order. Restore the ID page's order before rendering.
    let machines = page
        .machine_ids
        .into_iter()
        .map(|id| {
            by_id.remove(&Some(id)).ok_or_else(|| {
                tonic::Status::internal(format!(
                    "machine {id} was selected but its details are missing"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((machines, page.total_count))
}

async fn fetch_dpus(api: &Arc<Api>) -> Result<Vec<Row>, tonic::Status> {
    let mut machines = Vec::new();
    loop {
        let (page, _) = fetch_dpu_page(api, machines.len() as u64, 100).await?;
        let is_last_page = page.len() < 100;
        machines.extend(page.into_iter().map(Row::from));
        if is_last_page {
            return Ok(machines);
        }
    }
}

pub(super) async fn list_html(
    AxumState(state): AxumState<Arc<Api>>,
    Query(params): Query<PaginationParams>,
) -> impl IntoResponse {
    let (current_page, limit) = pagination::resolve_params(&params);
    let result = if limit == 0 {
        fetch_dpus(&state).await.map(|machines| {
            let page = PageContext::all(machines.len(), "/admin/dpu/versions");
            (machines, page)
        })
    } else {
        let offset = current_page.saturating_mul(limit).min(i64::MAX as usize) as u64;
        fetch_dpu_page(&state, offset, limit as u32)
            .await
            .map(|(machines, total_count)| {
                let page = PageContext::from_total(
                    current_page,
                    limit,
                    total_count as usize,
                    "/admin/dpu/versions",
                );
                let machines = machines.into_iter().map(Row::from).collect();
                (machines, page)
            })
    };
    let (machines, page) = match result {
        Ok(result) => result,
        Err(err) => {
            tracing::error!(error = %err, "fetch_dpus");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Error loading DPUs").into_response();
        }
    };
    let tmpl = DpuVersions { machines, page };
    (StatusCode::OK, Html(tmpl.render().unwrap())).into_response()
}

pub(super) async fn list_json(AxumState(state): AxumState<Arc<Api>>) -> impl IntoResponse {
    let machines = match fetch_dpus(&state).await {
        Ok(m) => m,
        Err(err) => {
            tracing::error!(error = %err, "fetch_dpus");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Error loading DPUs").into_response();
        }
    };
    (StatusCode::OK, Json(machines)).into_response()
}

impl super::Base for DpuVersions {}
