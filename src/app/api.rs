//! API request handling and response building for [`App`].

use super::{
    agent_name, detect_state_from_api, encode_api_keys, encode_api_text, pane_agent_status,
    parse_agent_name, tab_attention_priority, App, Mode,
};

pub(super) fn api_request_changes_ui(request: &crate::api::schema::Request) -> bool {
    use crate::api::schema::Method;

    matches!(
        &request.method,
        Method::WorkspaceCreate(_)
            | Method::WorkspaceFocus(_)
            | Method::WorkspaceRename(_)
            | Method::WorkspaceClose(_)
            | Method::TabCreate(_)
            | Method::TabFocus(_)
            | Method::TabRename(_)
            | Method::TabClose(_)
            | Method::PaneSplit(_)
            | Method::PaneReportAgent(_)
            | Method::PaneClearAgentAuthority(_)
            | Method::PaneReleaseAgent(_)
            | Method::PaneClose(_)
    )
}

impl App {
    pub(super) fn handle_api_request_message(
        &mut self,
        msg: crate::api::ApiRequestMessage,
    ) -> bool {
        let changed = api_request_changes_ui(&msg.request);
        let response = self.handle_api_request(msg.request);
        let _ = msg.respond_to.send(response);
        changed
    }

    pub(super) fn public_workspace_id(&self, ws_idx: usize) -> String {
        self.state.workspaces[ws_idx].id.clone()
    }

    pub(super) fn public_tab_id(&self, ws_idx: usize, tab_idx: usize) -> Option<String> {
        let ws = self.state.workspaces.get(ws_idx)?;
        ws.tabs.get(tab_idx)?;
        Some(format!("{}:{}", ws.id, tab_idx + 1))
    }

    pub(super) fn public_pane_id(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<String> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let pane_number = ws.public_pane_number(pane_id)?;
        Some(format!("{}-{pane_number}", ws.id))
    }

    pub(super) fn parse_workspace_id(&self, id: &str) -> Option<usize> {
        self.state
            .workspaces
            .iter()
            .position(|workspace| workspace.id == id)
            .or_else(|| id.strip_prefix("w_")?.parse::<usize>().ok()?.checked_sub(1))
            .or_else(|| id.parse::<usize>().ok()?.checked_sub(1))
    }

    pub(super) fn parse_tab_id(&self, id: &str) -> Option<(usize, usize)> {
        if let Some(rest) = id.strip_prefix("t_") {
            let (ws_raw, tab_raw) = rest.rsplit_once('_')?;
            let ws_idx = self.parse_workspace_id(ws_raw)?;
            let tab_idx = tab_raw.parse::<usize>().ok()?.checked_sub(1)?;
            self.state.workspaces.get(ws_idx)?.tabs.get(tab_idx)?;
            return Some((ws_idx, tab_idx));
        }

        let (ws_raw, tab_raw) = id.rsplit_once(':')?;
        let ws_idx = self.parse_workspace_id(ws_raw)?;
        let tab_idx = tab_raw.parse::<usize>().ok()?.checked_sub(1)?;
        self.state.workspaces.get(ws_idx)?.tabs.get(tab_idx)?;
        Some((ws_idx, tab_idx))
    }

    pub(super) fn parse_pane_id(&self, id: &str) -> Option<(usize, crate::layout::PaneId)> {
        if let Some(rest) = id.strip_prefix("p_") {
            if let Some((ws_raw, pane_raw)) = rest.rsplit_once('_') {
                let ws_idx = self.parse_workspace_id(ws_raw)?;
                let pane_id = crate::layout::PaneId::from_raw(pane_raw.parse::<u32>().ok()?);
                return Some((ws_idx, pane_id));
            }

            let pane_id = crate::layout::PaneId::from_raw(rest.parse::<u32>().ok()?);
            return self.find_pane(pane_id).map(|(ws_idx, _)| (ws_idx, pane_id));
        }

        let (ws_raw, pane_number_raw) = id.rsplit_once('-')?;
        let ws_idx = self.parse_workspace_id(ws_raw)?;
        let pane_number = pane_number_raw.parse::<usize>().ok()?;
        let ws = self.state.workspaces.get(ws_idx)?;
        let pane_id = ws
            .public_pane_numbers
            .iter()
            .find_map(|(pane_id, number)| (*number == pane_number).then_some(*pane_id))?;
        Some((ws_idx, pane_id))
    }

    pub(super) fn handle_api_request(&mut self, request: crate::api::schema::Request) -> String {
        self.drain_internal_events();
        use bytes::Bytes;

        use crate::api::schema::{
            ErrorBody, ErrorResponse, Method, PaneListParams, PaneReadResult, ReadSource,
            ResponseResult, SuccessResponse, TabListParams,
        };

        let response = match request.method {
            Method::WorkspaceList(_) => SuccessResponse {
                id: request.id,
                result: ResponseResult::WorkspaceList {
                    workspaces: self
                        .state
                        .workspaces
                        .iter()
                        .enumerate()
                        .map(|(idx, _)| self.workspace_info(idx))
                        .collect(),
                },
            },
            Method::WorkspaceGet(target) => {
                let Some(index) = self.parse_workspace_id(&target.workspace_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "workspace_not_found".into(),
                            message: format!("workspace {} not found", target.workspace_id),
                        },
                    })
                    .unwrap();
                };
                let Some(_) = self.state.workspaces.get(index) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "workspace_not_found".into(),
                            message: format!("workspace {} not found", target.workspace_id),
                        },
                    })
                    .unwrap();
                };
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::WorkspaceInfo {
                        workspace: self.workspace_info(index),
                    },
                }
            }
            Method::WorkspaceCreate(params) => {
                let cwd = params
                    .cwd
                    .map(std::path::PathBuf::from)
                    .or_else(|| std::env::current_dir().ok())
                    .unwrap_or_else(|| std::path::PathBuf::from("/"));
                match self.create_workspace_with_options(cwd, params.focus) {
                    Ok(index) => {
                        if let Some(label) = params.label {
                            if let Some(workspace) = self.state.workspaces.get_mut(index) {
                                workspace.set_custom_name(label);
                            }
                        }
                        let workspace = self.workspace_info(index);
                        let tab = self
                            .tab_info(index, 0)
                            .expect("new workspace should have an initial tab");
                        let root_pane = self
                            .root_pane_info(index, 0)
                            .expect("new workspace should have an initial root pane");
                        self.emit_event(crate::api::schema::EventEnvelope {
                            event: crate::api::schema::EventKind::WorkspaceCreated,
                            data: crate::api::schema::EventData::WorkspaceCreated {
                                workspace: workspace.clone(),
                            },
                        });
                        self.emit_event(crate::api::schema::EventEnvelope {
                            event: crate::api::schema::EventKind::TabCreated,
                            data: crate::api::schema::EventData::TabCreated { tab: tab.clone() },
                        });
                        self.emit_event(crate::api::schema::EventEnvelope {
                            event: crate::api::schema::EventKind::PaneCreated,
                            data: crate::api::schema::EventData::PaneCreated {
                                pane: root_pane.clone(),
                            },
                        });
                        SuccessResponse {
                            id: request.id,
                            result: self
                                .workspace_created_result(index)
                                .expect("new workspace should produce a complete create response"),
                        }
                    }
                    Err(err) => {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "workspace_create_failed".into(),
                                message: err.to_string(),
                            },
                        })
                        .unwrap();
                    }
                }
            }
            Method::WorkspaceFocus(target) => {
                let Some(index) = self.parse_workspace_id(&target.workspace_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "workspace_not_found".into(),
                            message: format!("workspace {} not found", target.workspace_id),
                        },
                    })
                    .unwrap();
                };
                if self.state.workspaces.get(index).is_none() {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "workspace_not_found".into(),
                            message: format!("workspace {} not found", target.workspace_id),
                        },
                    })
                    .unwrap();
                }
                self.state.switch_workspace(index);
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::WorkspaceInfo {
                        workspace: self.workspace_info(index),
                    },
                }
            }
            Method::WorkspaceRename(params) => {
                let Some(index) = self.parse_workspace_id(&params.workspace_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "workspace_not_found".into(),
                            message: format!("workspace {} not found", params.workspace_id),
                        },
                    })
                    .unwrap();
                };
                let Some(ws) = self.state.workspaces.get_mut(index) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "workspace_not_found".into(),
                            message: format!("workspace {} not found", params.workspace_id),
                        },
                    })
                    .unwrap();
                };
                ws.set_custom_name(params.label.clone());
                self.schedule_session_save();
                self.emit_event(crate::api::schema::EventEnvelope {
                    event: crate::api::schema::EventKind::WorkspaceRenamed,
                    data: crate::api::schema::EventData::WorkspaceRenamed {
                        workspace_id: self.public_workspace_id(index),
                        label: params.label,
                    },
                });
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::WorkspaceInfo {
                        workspace: self.workspace_info(index),
                    },
                }
            }
            Method::WorkspaceClose(target) => {
                let Some(index) = self.parse_workspace_id(&target.workspace_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "workspace_not_found".into(),
                            message: format!("workspace {} not found", target.workspace_id),
                        },
                    })
                    .unwrap();
                };
                if self.state.workspaces.get(index).is_none() {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "workspace_not_found".into(),
                            message: format!("workspace {} not found", target.workspace_id),
                        },
                    })
                    .unwrap();
                }
                self.state.selected = index;
                self.state.close_selected_workspace();
                self.emit_event(crate::api::schema::EventEnvelope {
                    event: crate::api::schema::EventKind::WorkspaceClosed,
                    data: crate::api::schema::EventData::WorkspaceClosed {
                        workspace_id: target.workspace_id,
                    },
                });
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::Ok {},
                }
            }
            Method::TabList(TabListParams { workspace_id }) => {
                let tabs = if let Some(workspace_id) = workspace_id {
                    let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "workspace_not_found".into(),
                                message: format!("workspace {} not found", workspace_id),
                            },
                        })
                        .unwrap();
                    };
                    let Some(ws) = self.state.workspaces.get(ws_idx) else {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "workspace_not_found".into(),
                                message: format!("workspace {} not found", workspace_id),
                            },
                        })
                        .unwrap();
                    };
                    (0..ws.tabs.len())
                        .filter_map(|tab_idx| self.tab_info(ws_idx, tab_idx))
                        .collect()
                } else {
                    let mut tabs = Vec::new();
                    for (ws_idx, ws) in self.state.workspaces.iter().enumerate() {
                        for tab_idx in 0..ws.tabs.len() {
                            if let Some(tab) = self.tab_info(ws_idx, tab_idx) {
                                tabs.push(tab);
                            }
                        }
                    }
                    tabs
                };
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::TabList { tabs },
                }
            }
            Method::TabGet(target) => {
                let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "tab_not_found".into(),
                            message: format!("tab {} not found", target.tab_id),
                        },
                    })
                    .unwrap();
                };
                let Some(tab) = self.tab_info(ws_idx, tab_idx) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "tab_not_found".into(),
                            message: format!("tab {} not found", target.tab_id),
                        },
                    })
                    .unwrap();
                };
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::TabInfo { tab },
                }
            }
            Method::TabCreate(params) => {
                let crate::api::schema::TabCreateParams {
                    workspace_id,
                    cwd,
                    focus,
                    label,
                } = params;
                let ws_idx = if let Some(workspace_id) = workspace_id {
                    let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "workspace_not_found".into(),
                                message: format!("workspace {} not found", workspace_id),
                            },
                        })
                        .unwrap();
                    };
                    ws_idx
                } else if let Some(active) = self.state.active {
                    active
                } else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "workspace_not_found".into(),
                            message: "no active workspace".into(),
                        },
                    })
                    .unwrap();
                };
                let cwd = cwd
                    .map(std::path::PathBuf::from)
                    .or_else(|| {
                        self.state.workspaces.get(ws_idx).and_then(|ws| {
                            ws.active_tab()
                                .and_then(|tab| tab.focused_runtime())
                                .and_then(|rt| rt.cwd())
                        })
                    })
                    .or_else(|| std::env::current_dir().ok())
                    .unwrap_or_else(|| std::path::PathBuf::from("/"));
                let (rows, cols) = self.state.estimate_pane_size();
                let result = self
                    .state
                    .workspaces
                    .get_mut(ws_idx)
                    .ok_or_else(|| std::io::Error::other("workspace disappeared"))
                    .and_then(|ws| {
                        ws.create_tab(
                            rows,
                            cols,
                            cwd,
                            self.state.pane_scrollback_limit_bytes,
                            self.state.host_terminal_theme,
                        )
                    });
                match result {
                    Ok(tab_idx) => {
                        if let Some(label) = label {
                            if let Some(tab) = self
                                .state
                                .workspaces
                                .get_mut(ws_idx)
                                .and_then(|ws| ws.tabs.get_mut(tab_idx))
                            {
                                tab.set_custom_name(label);
                            }
                        }
                        if focus {
                            self.state.switch_workspace(ws_idx);
                            self.state.switch_tab(tab_idx);
                            self.state.mode = Mode::Terminal;
                        }
                        self.schedule_session_save();
                        let tab = self.tab_info(ws_idx, tab_idx).unwrap();
                        let root_pane = self
                            .root_pane_info(ws_idx, tab_idx)
                            .expect("new tab should have a root pane");
                        self.emit_event(crate::api::schema::EventEnvelope {
                            event: crate::api::schema::EventKind::TabCreated,
                            data: crate::api::schema::EventData::TabCreated { tab: tab.clone() },
                        });
                        self.emit_event(crate::api::schema::EventEnvelope {
                            event: crate::api::schema::EventKind::PaneCreated,
                            data: crate::api::schema::EventData::PaneCreated {
                                pane: root_pane.clone(),
                            },
                        });
                        SuccessResponse {
                            id: request.id,
                            result: self
                                .tab_created_result(ws_idx, tab_idx)
                                .expect("new tab should produce a complete create response"),
                        }
                    }
                    Err(err) => {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "tab_create_failed".into(),
                                message: err.to_string(),
                            },
                        })
                        .unwrap();
                    }
                }
            }
            Method::TabFocus(target) => {
                let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "tab_not_found".into(),
                            message: format!("tab {} not found", target.tab_id),
                        },
                    })
                    .unwrap();
                };
                self.state.switch_workspace(ws_idx);
                self.state.switch_tab(tab_idx);
                let tab = self.tab_info(ws_idx, tab_idx).unwrap();
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::TabInfo { tab },
                }
            }
            Method::TabRename(params) => {
                let Some((ws_idx, tab_idx)) = self.parse_tab_id(&params.tab_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "tab_not_found".into(),
                            message: format!("tab {} not found", params.tab_id),
                        },
                    })
                    .unwrap();
                };
                let Some(tab) = self
                    .state
                    .workspaces
                    .get_mut(ws_idx)
                    .and_then(|ws| ws.tabs.get_mut(tab_idx))
                else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "tab_not_found".into(),
                            message: format!("tab {} not found", params.tab_id),
                        },
                    })
                    .unwrap();
                };
                tab.set_custom_name(params.label.clone());
                self.schedule_session_save();
                self.emit_event(crate::api::schema::EventEnvelope {
                    event: crate::api::schema::EventKind::TabRenamed,
                    data: crate::api::schema::EventData::TabRenamed {
                        tab_id: self.public_tab_id(ws_idx, tab_idx).unwrap(),
                        workspace_id: self.public_workspace_id(ws_idx),
                        label: params.label,
                    },
                });
                let tab = self.tab_info(ws_idx, tab_idx).unwrap();
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::TabInfo { tab },
                }
            }
            Method::TabClose(target) => {
                let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "tab_not_found".into(),
                            message: format!("tab {} not found", target.tab_id),
                        },
                    })
                    .unwrap();
                };
                let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "tab_not_found".into(),
                            message: format!("tab {} not found", target.tab_id),
                        },
                    })
                    .unwrap();
                };
                if ws.tabs.len() <= 1 {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "tab_close_failed".into(),
                            message: "cannot close the last tab in a workspace".into(),
                        },
                    })
                    .unwrap();
                }
                if !ws.close_tab(tab_idx) {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "tab_close_failed".into(),
                            message: format!("tab {} could not be closed", target.tab_id),
                        },
                    })
                    .unwrap();
                }
                self.schedule_session_save();
                self.emit_event(crate::api::schema::EventEnvelope {
                    event: crate::api::schema::EventKind::TabClosed,
                    data: crate::api::schema::EventData::TabClosed {
                        tab_id: target.tab_id,
                        workspace_id: self.public_workspace_id(ws_idx),
                    },
                });
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::Ok {},
                }
            }
            Method::PaneSplit(params) => {
                let Some((ws_idx, target_pane_id)) = self.parse_pane_id(&params.target_pane_id)
                else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.target_pane_id),
                        },
                    })
                    .unwrap();
                };
                let (rows, cols) = self.state.estimate_pane_size();
                let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.target_pane_id),
                        },
                    })
                    .unwrap();
                };
                ws.layout.focus_pane(target_pane_id);
                let direction = match params.direction {
                    crate::api::schema::SplitDirection::Right => {
                        ratatui::layout::Direction::Horizontal
                    }
                    crate::api::schema::SplitDirection::Down => {
                        ratatui::layout::Direction::Vertical
                    }
                };
                let new_pane_id = match ws.split_focused(
                    direction,
                    rows,
                    cols,
                    params.cwd.map(std::path::PathBuf::from),
                    self.state.pane_scrollback_limit_bytes,
                    self.state.host_terminal_theme,
                ) {
                    Ok(new_pane_id) => new_pane_id,
                    Err(err) => {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "pane_split_failed".into(),
                                message: err.to_string(),
                            },
                        })
                        .unwrap();
                    }
                };
                if !params.focus {
                    ws.layout.focus_pane(target_pane_id);
                }
                self.schedule_session_save();
                let pane = self.pane_info(ws_idx, new_pane_id).unwrap();
                self.emit_event(crate::api::schema::EventEnvelope {
                    event: crate::api::schema::EventKind::PaneCreated,
                    data: crate::api::schema::EventData::PaneCreated { pane: pane.clone() },
                });
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::PaneInfo { pane },
                }
            }
            Method::PaneList(PaneListParams { workspace_id }) => {
                match self.collect_panes_for_workspace(workspace_id.as_deref()) {
                    Ok(panes) => SuccessResponse {
                        id: request.id,
                        result: ResponseResult::PaneList { panes },
                    },
                    Err((code, message)) => {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody { code, message },
                        })
                        .unwrap();
                    }
                }
            }
            Method::PaneGet(target) => {
                let Some((ws_idx, pane_id)) = self.parse_pane_id(&target.pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", target.pane_id),
                        },
                    })
                    .unwrap();
                };
                let Some(pane) = self.pane_info(ws_idx, pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", target.pane_id),
                        },
                    })
                    .unwrap();
                };
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::PaneInfo { pane },
                }
            }
            Method::PaneRead(params) => {
                let Some((ws_idx, pane_id)) = self.parse_pane_id(&params.pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                let Some((pane, workspace_id)) = self.lookup_runtime(ws_idx, pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                let Some(tab_idx) = self
                    .state
                    .workspaces
                    .get(ws_idx)
                    .and_then(|ws| ws.find_tab_index_for_pane(pane_id))
                else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                let requested_lines = params.lines.unwrap_or(80).min(1000) as usize;
                let text = match params.source {
                    ReadSource::Visible => pane.visible_text(),
                    ReadSource::Recent => pane.recent_text(requested_lines),
                    ReadSource::RecentUnwrapped => pane.recent_unwrapped_text(requested_lines),
                };
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::PaneRead {
                        read: PaneReadResult {
                            pane_id: params.pane_id,
                            workspace_id,
                            tab_id: self.public_tab_id(ws_idx, tab_idx).unwrap(),
                            source: params.source,
                            text,
                            revision: 0,
                            truncated: false,
                        },
                    },
                }
            }
            Method::PaneReportAgent(params) => {
                let Some((_ws_idx, pane_id)) = self.parse_pane_id(&params.pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                let Some(agent) = parse_agent_name(&params.agent) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "invalid_agent".into(),
                            message: format!("unsupported agent {}", params.agent),
                        },
                    })
                    .unwrap();
                };
                self.handle_internal_event(crate::events::AppEvent::HookStateReported {
                    pane_id,
                    source: params.source,
                    agent,
                    state: detect_state_from_api(params.state),
                    message: params.message,
                });
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::Ok {},
                }
            }
            Method::PaneClearAgentAuthority(params) => {
                let Some((_ws_idx, pane_id)) = self.parse_pane_id(&params.pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                self.handle_internal_event(crate::events::AppEvent::HookAuthorityCleared {
                    pane_id,
                    source: params.source,
                });
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::Ok {},
                }
            }
            Method::PaneReleaseAgent(params) => {
                let Some((_ws_idx, pane_id)) = self.parse_pane_id(&params.pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                let Some(agent) = parse_agent_name(&params.agent) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "invalid_agent".into(),
                            message: format!("unsupported agent {}", params.agent),
                        },
                    })
                    .unwrap();
                };
                self.handle_internal_event(crate::events::AppEvent::HookAgentReleased {
                    pane_id,
                    source: params.source,
                    agent,
                });
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::Ok {},
                }
            }
            Method::PaneSendText(params) => {
                let Some((ws_idx, pane_id)) = self.parse_pane_id(&params.pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                let Some(runtime) = self.lookup_runtime_sender(ws_idx, pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                if let Err(err) = runtime.try_send_bytes(Bytes::from(params.text)) {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_send_failed".into(),
                            message: err.to_string(),
                        },
                    })
                    .unwrap();
                }
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::Ok {},
                }
            }
            Method::PaneSendInput(params) => {
                let Some((ws_idx, pane_id)) = self.parse_pane_id(&params.pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                let Some(runtime) = self.lookup_runtime_sender(ws_idx, pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                let encoded_keys = match encode_api_keys(runtime, &params.keys) {
                    Ok(encoded_keys) => encoded_keys,
                    Err(key) => {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "invalid_key".into(),
                                message: format!("unsupported key {key}"),
                            },
                        })
                        .unwrap();
                    }
                };
                if !params.text.is_empty() {
                    let text_bytes = encode_api_text(runtime, &params.text);
                    if let Err(err) = runtime.try_send_bytes(Bytes::from(text_bytes)) {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "pane_send_failed".into(),
                                message: err.to_string(),
                            },
                        })
                        .unwrap();
                    }
                }
                for bytes in encoded_keys {
                    if let Err(err) = runtime.try_send_bytes(Bytes::from(bytes)) {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "pane_send_failed".into(),
                                message: err.to_string(),
                            },
                        })
                        .unwrap();
                    }
                }
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::Ok {},
                }
            }
            Method::PaneClose(target) => {
                let Some((ws_idx, pane_id)) = self.parse_pane_id(&target.pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", target.pane_id),
                        },
                    })
                    .unwrap();
                };
                let workspace_id = self.state.workspaces[ws_idx].id.clone();
                let should_close_workspace = {
                    let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "pane_not_found".into(),
                                message: format!("pane {} not found", target.pane_id),
                            },
                        })
                        .unwrap();
                    };
                    ws.close_pane(pane_id)
                };
                if should_close_workspace {
                    self.state.selected = ws_idx;
                    self.state.close_selected_workspace();
                    self.emit_event(crate::api::schema::EventEnvelope {
                        event: crate::api::schema::EventKind::PaneClosed,
                        data: crate::api::schema::EventData::PaneClosed {
                            pane_id: target.pane_id.clone(),
                            workspace_id: workspace_id.clone(),
                        },
                    });
                    self.emit_event(crate::api::schema::EventEnvelope {
                        event: crate::api::schema::EventKind::WorkspaceClosed,
                        data: crate::api::schema::EventData::WorkspaceClosed { workspace_id },
                    });
                } else {
                    self.schedule_session_save();
                    self.emit_event(crate::api::schema::EventEnvelope {
                        event: crate::api::schema::EventKind::PaneClosed,
                        data: crate::api::schema::EventData::PaneClosed {
                            pane_id: target.pane_id,
                            workspace_id,
                        },
                    });
                }
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::Ok {},
                }
            }
            Method::PaneSendKeys(params) => {
                let Some((ws_idx, pane_id)) = self.parse_pane_id(&params.pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                let Some(runtime) = self.lookup_runtime_sender(ws_idx, pane_id) else {
                    return serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: format!("pane {} not found", params.pane_id),
                        },
                    })
                    .unwrap();
                };
                let encoded_keys = match encode_api_keys(runtime, &params.keys) {
                    Ok(encoded_keys) => encoded_keys,
                    Err(key) => {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "invalid_key".into(),
                                message: format!("unsupported key {key}"),
                            },
                        })
                        .unwrap();
                    }
                };
                for bytes in encoded_keys {
                    if let Err(err) = runtime.try_send_bytes(Bytes::from(bytes)) {
                        return serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error: ErrorBody {
                                code: "pane_send_failed".into(),
                                message: err.to_string(),
                            },
                        })
                        .unwrap();
                    }
                }
                SuccessResponse {
                    id: request.id,
                    result: ResponseResult::Ok {},
                }
            }
            _ => {
                return serde_json::to_string(&ErrorResponse {
                    id: request.id,
                    error: ErrorBody {
                        code: "not_implemented".into(),
                        message: "method not implemented yet".into(),
                    },
                })
                .unwrap();
            }
        };

        serde_json::to_string(&response).unwrap()
    }

    pub(super) fn collect_panes_for_workspace(
        &self,
        workspace_id: Option<&str>,
    ) -> Result<Vec<crate::api::schema::PaneInfo>, (String, String)> {
        if let Some(workspace_id) = workspace_id {
            let Some(ws_idx) = self.parse_workspace_id(workspace_id) else {
                return Err((
                    "workspace_not_found".into(),
                    format!("workspace {workspace_id} not found"),
                ));
            };
            let Some(ws) = self.state.workspaces.get(ws_idx) else {
                return Err((
                    "workspace_not_found".into(),
                    format!("workspace {workspace_id} not found"),
                ));
            };
            Ok(ws
                .tabs
                .iter()
                .flat_map(|tab| tab.layout.pane_ids().into_iter())
                .filter_map(|pane_id| self.pane_info(ws_idx, pane_id))
                .collect())
        } else {
            Ok(self
                .state
                .workspaces
                .iter()
                .enumerate()
                .flat_map(|(ws_idx, ws)| {
                    ws.tabs
                        .iter()
                        .flat_map(|tab| tab.layout.pane_ids().into_iter())
                        .filter_map(move |pane_id| self.pane_info(ws_idx, pane_id))
                })
                .collect())
        }
    }

    pub(super) fn tab_info(
        &self,
        ws_idx: usize,
        tab_idx: usize,
    ) -> Option<crate::api::schema::TabInfo> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let tab = ws.tabs.get(tab_idx)?;
        let (agg_state, seen) = tab
            .panes
            .values()
            .map(|pane| (pane.state, pane.seen))
            .max_by_key(|(state, seen)| tab_attention_priority(*state, *seen))
            .unwrap_or((crate::detect::AgentState::Unknown, true));
        Some(crate::api::schema::TabInfo {
            tab_id: self.public_tab_id(ws_idx, tab_idx)?,
            workspace_id: self.public_workspace_id(ws_idx),
            number: tab_idx + 1,
            label: tab.display_name(),
            focused: self.state.active == Some(ws_idx) && ws.active_tab == tab_idx,
            pane_count: tab.panes.len(),
            agent_status: pane_agent_status(agg_state, seen),
        })
    }

    pub(super) fn workspace_created_result(
        &self,
        ws_idx: usize,
    ) -> Option<crate::api::schema::ResponseResult> {
        Some(crate::api::schema::ResponseResult::WorkspaceCreated {
            workspace: self.workspace_info(ws_idx),
            tab: self.tab_info(ws_idx, 0)?,
            root_pane: self.root_pane_info(ws_idx, 0)?,
        })
    }

    pub(super) fn tab_created_result(
        &self,
        ws_idx: usize,
        tab_idx: usize,
    ) -> Option<crate::api::schema::ResponseResult> {
        Some(crate::api::schema::ResponseResult::TabCreated {
            tab: self.tab_info(ws_idx, tab_idx)?,
            root_pane: self.root_pane_info(ws_idx, tab_idx)?,
        })
    }

    pub(super) fn root_pane_info(
        &self,
        ws_idx: usize,
        tab_idx: usize,
    ) -> Option<crate::api::schema::PaneInfo> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let tab = ws.tabs.get(tab_idx)?;
        self.pane_info(ws_idx, tab.root_pane)
    }

    pub(super) fn pane_info(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<crate::api::schema::PaneInfo> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let pane = ws.pane_state(pane_id)?;
        let runtime = ws.runtime(pane_id);
        let tab_idx = ws.find_tab_index_for_pane(pane_id)?;
        let focused = self.state.active == Some(ws_idx)
            && ws.active_tab == tab_idx
            && ws
                .focused_pane_id()
                .is_some_and(|focused| focused == pane_id);
        Some(crate::api::schema::PaneInfo {
            pane_id: self.public_pane_id(ws_idx, pane_id)?,
            workspace_id: self.public_workspace_id(ws_idx),
            tab_id: self.public_tab_id(ws_idx, tab_idx)?,
            focused,
            cwd: runtime
                .and_then(|rt| rt.cwd())
                .map(|cwd| cwd.display().to_string()),
            agent: pane.detected_agent.map(agent_name),
            agent_status: pane_agent_status(pane.state, pane.seen),
            revision: 0,
        })
    }

    pub(super) fn lookup_runtime(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<(&crate::pane::PaneRuntime, String)> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let runtime = ws.runtime(pane_id)?;
        Some((runtime, self.public_workspace_id(ws_idx)))
    }

    pub(super) fn lookup_runtime_sender(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<&crate::pane::PaneRuntime> {
        let ws = self.state.workspaces.get(ws_idx)?;
        ws.runtime(pane_id)
    }

    pub(super) fn workspace_info(&self, index: usize) -> crate::api::schema::WorkspaceInfo {
        let ws = &self.state.workspaces[index];
        let (agg_state, seen) = ws.aggregate_state();
        crate::api::schema::WorkspaceInfo {
            workspace_id: self.public_workspace_id(index),
            number: index + 1,
            label: ws.display_name(),
            focused: self.state.active == Some(index),
            pane_count: ws.public_pane_numbers.len(),
            tab_count: ws.tabs.len(),
            active_tab_id: self
                .public_tab_id(index, ws.active_tab)
                .unwrap_or_else(|| format!("{}:{}", ws.id, ws.active_tab + 1)),
            agent_status: pane_agent_status(agg_state, seen),
        }
    }
}
