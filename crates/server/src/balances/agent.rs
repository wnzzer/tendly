use super::*;
use crate::{
    activity::Answer,
    agent::{Caller, text},
};

pub(crate) fn tool() -> Value {
    crate::agent::tool(
        "tendly_balance",
        "Configure or query an endpoint's custom balance. get reads configuration and last result; configure saves or disables it after browser confirmation; query refreshes the approved script (15-second cooldown); test executes only the saved script without saving a result. Script is an async JS body: const data = await ctx.request(); return {value: data.balance}; ctx.request({url?,method?,headers?,body?}) returns JSON, allows GET/POST and only the configured query origin. Keys stay on the server. Money uses its configured currency without conversion; percentage always means remaining, 0–100. Save keys through Tendly, never put credentials in scripts or chat.",
        json!({"type":"object","additionalProperties":false,"required":["action","id"],"properties":{
            "action":{"type":"string","enum":["get","configure","query","test"]},
            "id":{"type":"string","description":"Endpoint profile ID from tendly_list."},
            "config":{"description":"Required for configure. null disables the query.","anyOf":[{"type":"null"},{"type":"object","additionalProperties":false,"required":["unit","query_url","auth","script"],"properties":{
                "unit":{"oneOf":[{"type":"object","additionalProperties":false,"required":["kind","currency"],"properties":{"kind":{"const":"money"},"currency":{"type":"string","pattern":"^[A-Z]{3}$"}}},{"type":"object","additionalProperties":false,"required":["kind"],"properties":{"kind":{"const":"percentage"}}}]},
                "query_url":{"type":"string","description":"HTTP(S) URL for the balance API. Authentication is sent only to this origin."},
                "auth":{"description":"Where the server puts the endpoint key; scripts never see it.","oneOf":[
                    {"type":"object","additionalProperties":false,"required":["kind"],"properties":{"kind":{"const":"none"}}},
                    {"type":"object","additionalProperties":false,"required":["kind","name","value"],"properties":{"kind":{"const":"header"},"name":{"type":"string","description":"Header name, e.g. Authorization or X-Token."},"value":{"type":"string","description":"Contains {key} exactly once, e.g. \"Bearer {key}\"."}}},
                    {"type":"object","additionalProperties":false,"required":["kind","name"],"properties":{"kind":{"const":"query"},"name":{"type":"string","description":"Query parameter name, e.g. api_key."}}}
                ]},
                "secret_ref":{"type":["string","null"],"description":"Saved env:TENDLY_SECRET_ reference. Omit to use the endpoint's existing key."},
                "auto_refresh":{"type":"boolean","description":"Whether an open Tendly page queries it again every 5 minutes. Omit for on."},
                "script":{"type":"string","maxLength":32768}
            }}]}
        }}),
    )
}
pub(crate) async fn call(
    state: &AppState,
    caller: Caller,
    args: &Value,
) -> Result<Value, ApiError> {
    let id: Uuid = text(args, "id")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ApiError::bad("A valid endpoint profile id is required"))?;
    let profile = profile(state, id).await?;
    match text(args, "action") {
        Some("get") => Ok(json!(View::from(load(state, id).await?.0))),
        Some("query") => Ok(json!(query(state, id).await?)),
        Some("test") => {
            let config = load(state, id).await?.0.config.ok_or_else(|| {
                ApiError::bad("Save a balance configuration before testing it through MCP")
            })?;
            Ok(json!(
                execute(state, profile, config)
                    .await
                    .map_err(ApiError::bad)?
            ))
        }
        Some("configure") => {
            let config: Option<Config> = serde_json::from_value(
                args.get("config")
                    .cloned()
                    .ok_or_else(|| ApiError::bad("config is required; null disables the query"))?,
            )
            .map_err(|_| ApiError::bad("Invalid balance configuration"))?;
            if let Some(config) = &config {
                validate(config)?;
            }
            let rows = match &config {
                Some(config) => vec![
                    ("Balance type", serde_json::to_string(&config.unit).unwrap()),
                    ("Balance query URL", config.query_url.clone()),
                    (
                        "Authentication",
                        match &config.auth {
                            Auth::None {} => "None".into(),
                            Auth::Header { name, value } => format!("Header {name}: {value}"),
                            Auth::Query { name } => {
                                format!("Query parameter ?{name}={KEY_PLACEHOLDER}")
                            }
                        },
                    ),
                    (
                        "Key reference",
                        config
                            .secret_ref
                            .clone()
                            .unwrap_or_else(|| "Endpoint key".into()),
                    ),
                    ("JavaScript", config.script.clone()),
                ],
                None => vec![("Balance query", "Disabled".into())],
            };
            let who = crate::agent_config::who(state, caller).await;
            if !matches!(state.activity.ask(who.clone(),"Configure balance for “{name}”",json!({"name":profile.name}),rows,
                vec!["The saved key is sent to the configured balance query origin. Scripts can make up to four requests per run."],None).await?,Answer::Approved{..}) {
                return Err(ApiError::conflict("The person declined. Nothing was changed."));
            }
            let view = save_config(state, id, config).await?;
            state.activity.notify(
                who,
                "Updated balance query for “{name}”",
                json!({"name":profile.name}),
                None,
            );
            Ok(json!(view))
        }
        _ => Err(ApiError::bad(
            "action must be get, configure, query or test",
        )),
    }
}
