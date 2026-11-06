use super::*;
use roon_api::zone::LoopMode;
use roon_api::Zone;

#[derive(Deserialize)]
pub(super) struct TransportRequest {
    core_id: String,
    #[serde(flatten)]
    action: Action,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Settings {
        zone_id: String,
        shuffle: Option<bool>,
        loop_mode: Option<LoopMode>,
        auto_radio: Option<bool>,
    },
    Group {
        output_ids: Vec<String>,
    },
    Ungroup {
        output_ids: Vec<String>,
    },
    Transfer {
        from_zone_id: String,
        to_zone_id: String,
    },
    Source {
        output_id: String,
        control_key: String,
        command: SourceAction,
    },
    RelativeVolume {
        output_id: String,
        steps: i32,
    },
    SeekRelative {
        zone_id: String,
        seconds: i64,
    },
    PauseAll,
    Refresh,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceAction {
    Standby,
    ToggleStandby,
    ConvenienceSwitch,
}

pub(super) async fn transport_action(
    State(player): State<Player>,
    Json(input): Json<TransportRequest>,
) -> Result<Json<Value>, ApiError> {
    let (peer, zones) = player.selected().await?;
    if peer.endpoint.core_id != input.core_id {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Roon Server changed; try again",
        ));
    }
    let mut outputs: BTreeMap<String, Output> = {
        let state = player.inner.state.read().await;
        state
            .cores
            .get(&input.core_id)
            .into_iter()
            .flat_map(|entry| entry.outputs.values())
            .map(|value| {
                serde_json::from_value::<Output>(value.clone())
                    .map(|output| (output.output_id.clone(), output))
            })
            .collect::<std::result::Result<_, _>>()
            .context("Invalid Roon output data")?
    };
    for value in &zones {
        let zone: Zone = serde_json::from_value(value.clone()).context("Invalid Roon zone data")?;
        for output in zone.outputs {
            outputs.entry(output.output_id.clone()).or_insert(output);
        }
    }
    let transport = peer.transport();
    match input.action {
        Action::Settings {
            zone_id,
            shuffle,
            loop_mode,
            auto_radio,
        } => {
            let zone: Zone = serde_json::from_value(selected_zone(&zones, &zone_id)?.clone())
                .context("Invalid Roon zone")?;
            if zone.settings.is_none()
                || (shuffle.is_none() && loop_mode.is_none() && auto_radio.is_none())
            {
                return Err(ApiError::bad("Playback settings are not available"));
            }
            let mode = loop_mode.map(|mode| match mode {
                LoopMode::Loop => "loop",
                LoopMode::LoopOne => "loop_one",
                LoopMode::Disabled => "disabled",
            });
            peer.command(transport.change_settings(&zone_id, shuffle, mode, auto_radio))
                .await?;
        }
        Action::Group { output_ids } => {
            let selected = checked_outputs(&outputs, &output_ids, 2)?;
            for output in &selected {
                if selected.iter().any(|other| {
                    other.output_id != output.output_id
                        && !output.can_group_with_output_ids.contains(&other.output_id)
                }) {
                    return Err(ApiError::bad("These outputs cannot be grouped together"));
                }
            }
            peer.command(
                transport.group_outputs(&output_ids.iter().map(String::as_str).collect::<Vec<_>>()),
            )
            .await?;
        }
        Action::Ungroup { output_ids } => {
            checked_outputs(&outputs, &output_ids, 1)?;
            let grouped = zones.iter().any(|zone| {
                zone["outputs"].as_array().is_some_and(|members| {
                    members.len() > 1
                        && output_ids.iter().all(|id| {
                            members
                                .iter()
                                .any(|member| member["output_id"].as_str() == Some(id))
                        })
                })
            });
            if !grouped {
                return Err(ApiError::bad(
                    "Outputs must belong to the same grouped zone",
                ));
            }
            peer.command(
                transport
                    .ungroup_outputs(&output_ids.iter().map(String::as_str).collect::<Vec<_>>()),
            )
            .await?;
        }
        Action::Transfer {
            from_zone_id,
            to_zone_id,
        } => {
            selected_zone(&zones, &from_zone_id)?;
            selected_zone(&zones, &to_zone_id)?;
            if from_zone_id == to_zone_id {
                return Err(ApiError::bad("Choose a different destination zone"));
            }
            peer.command(transport.transfer_zone(&from_zone_id, &to_zone_id))
                .await?;
        }
        Action::Source {
            output_id,
            control_key,
            command,
        } => {
            let output = outputs
                .get(&output_id)
                .ok_or_else(|| ApiError::bad("Unknown audio output"))?;
            let source = output
                .source_controls
                .iter()
                .find(|source| source.control_key == control_key)
                .ok_or_else(|| ApiError::bad("Unknown source control"))?;
            if !matches!(command, SourceAction::ConvenienceSwitch) && !source.supports_standby {
                return Err(ApiError::bad("This source does not support standby"));
            }
            match command {
                SourceAction::Standby => {
                    peer.command(transport.standby(&output_id, Some(&control_key)))
                        .await?
                }
                SourceAction::ToggleStandby => {
                    peer.command(transport.toggle_standby(&output_id, Some(&control_key)))
                        .await?
                }
                SourceAction::ConvenienceSwitch => {
                    peer.command(transport.convenience_switch(&output_id, Some(&control_key)))
                        .await?
                }
            }
        }
        Action::RelativeVolume { output_id, steps } => {
            let output = outputs
                .get(&output_id)
                .ok_or_else(|| ApiError::bad("Unknown audio output"))?;
            if !(-10..=10).contains(&steps)
                || steps == 0
                || !output.volume.as_ref().is_some_and(|volume| {
                    matches!(volume.volume_type.as_str(), "number" | "db" | "incremental")
                })
            {
                return Err(ApiError::bad("Invalid volume step or unsupported output"));
            }
            peer.command(transport.change_volume(
                &output_id,
                VolumeMode::RelativeStep,
                f64::from(steps),
            ))
            .await?;
        }
        Action::SeekRelative { zone_id, seconds } => {
            let zone = selected_zone(&zones, &zone_id)?;
            if zone["is_seek_allowed"] != true || !(-60..=60).contains(&seconds) || seconds == 0 {
                return Err(ApiError::bad(
                    "Relative seek is unavailable or outside the allowed range",
                ));
            }
            peer.command(transport.seek(&zone_id, SeekMode::Relative, seconds))
                .await?;
        }
        Action::PauseAll => peer.command(transport.pause_all()).await?,
        Action::Refresh => {
            let zones = peer.command(transport.get_zones()).await?;
            let outputs = peer.command(transport.get_outputs()).await?;
            let mut state = player.inner.state.write().await;
            if let Some(entry) = state.cores.get_mut(&input.core_id) {
                if entry
                    .peer
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &peer))
                {
                    apply_zones(&mut entry.zones, "Subscribed", &json!({"zones":zones}));
                    apply_outputs(
                        &mut entry.outputs,
                        "Subscribed",
                        &json!({"outputs":outputs}),
                    );
                }
            }
        }
    }
    Ok(Json(json!({"ok":true})))
}

fn checked_outputs<'data>(
    outputs: &'data BTreeMap<String, Output>,
    ids: &[String],
    minimum: usize,
) -> Result<Vec<&'data Output>, ApiError> {
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    if ids.len() < minimum || ids.len() > 32 || unique.len() != ids.len() {
        return Err(ApiError::bad("Invalid output selection"));
    }
    ids.iter()
        .map(|id| {
            outputs
                .get(id)
                .ok_or_else(|| ApiError::bad("Unknown audio output"))
        })
        .collect()
}
