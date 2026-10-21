use super::*;
use std::io::Write;

fn write(path: &Path, lines: &[Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

#[test]
fn transcripts_of_both_clients_are_counted_once_each() {
    let home = std::env::temp_dir().join(format!("tendly-usage-{}", uuid::Uuid::new_v4()));
    let claude = home.join("projects/-repo/c1.jsonl");
    let reply = serde_json::json!({"type":"assistant","timestamp":"2026-10-01T10:00:00Z","sessionId":"c1","cwd":"/repo","requestId":"r1",
        "message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":10,"cache_read_input_tokens":1000,"output_tokens":100,
        "cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":200},"output_tokens_details":{"thinking_tokens":40}}}});
    // The same reply logged twice is one call.
    write(&claude, &[reply.clone(), reply]);
    let codex = home.join("sessions/2026/10/01/rollout.jsonl");
    let count = |total: u64, input: u64| {
        serde_json::json!({"timestamp":"2026-10-01T11:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{
        "total_token_usage":{"total_tokens":total},"last_token_usage":{"input_tokens":input,"cached_input_tokens":60,"output_tokens":5,"reasoning_output_tokens":2}}}})
    };
    write(
        &codex,
        &[
            serde_json::json!({"timestamp":"2026-10-01T11:00:00Z","type":"session_meta","payload":{"id":"x1","cwd":"/repo"}}),
            serde_json::json!({"timestamp":"2026-10-01T11:00:00Z","type":"turn_context","payload":{"model":"house-model-x","cwd":"/repo"}}),
            count(105, 100),
            // A repeated running total is a rate-limit update, not a call.
            count(105, 100),
        ],
    );
    let homes = vec![(home.clone(), None)];
    let calls = collect(&homes);
    assert_eq!(calls.len(), 2);
    let from = DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
        .unwrap()
        .timestamp();
    let mut links = HashMap::new();
    links.insert(
        "c1".to_string(),
        SessionLink {
            id: "s1".into(),
            title: "Fixture".into(),
        },
    );
    let result = report(
        &calls,
        from,
        from + 86_400,
        480,
        &links,
        &Prices::default(),
        &Filter::default(),
    );
    assert_eq!(result.totals.calls, 2);
    assert_eq!(result.totals.tokens.cache_read, 1060);
    assert_eq!(
        result.totals.tokens.input,
        10 + 40,
        "Codex's cached input is split out of its input"
    );
    assert_eq!(result.totals.tokens.cache_write_1h, 200);
    // Opus 5.5: 10 x $4 + 200 x $8 + 1000 x $0.20 + 100 x $20, per million.
    assert!((result.totals.cost - (40.0 + 1600.0 + 200.0 + 2000.0) / 1e6).abs() < 1e-12);
    assert_eq!(
        result.totals.unpriced,
        40 + 60 + 5,
        "a model without a published price has no cost"
    );
    let linked = result
        .conversations
        .iter()
        .find(|row| row.conversation == "c1")
        .unwrap();
    assert_eq!(linked.session_id.as_deref(), Some("s1"));
    assert_eq!(
        result.days.len(),
        2,
        "both local days the range touches have a row"
    );

    // An appended line is all a second read costs, and it is counted.
    write(&codex, &[count(300, 150)]);
    assert_eq!(collect(&homes).len(), 3);
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn messages_activity_and_filters() {
    let home = std::env::temp_dir().join(format!("tendly-usage-{}", uuid::Uuid::new_v4()));
    let claude = home.join("projects/-repo/c2.jsonl");
    let user = |uuid: &str, at: &str, content: Value| serde_json::json!({"type":"user","uuid":uuid,"timestamp":at,"sessionId":"c2","cwd":"/repo/app","message":{"role":"user","content":content}});
    let reply = |id: &str, at: &str| {
        serde_json::json!({"type":"assistant","timestamp":at,"sessionId":"c2","cwd":"/repo/app","requestId":id,
        "message":{"id":id,"model":"claude-sonnet-5-5","usage":{"input_tokens":100,"output_tokens":10}}})
    };
    write(
        &claude,
        &[
            user(
                "u1",
                "2026-10-01T10:00:00Z",
                Value::String("Build it".into()),
            ),
            reply("m1", "2026-10-01T10:01:00Z"),
            // A tool result comes back as a user line; the person did not type it.
            user(
                "u2",
                "2026-10-01T10:02:00Z",
                serde_json::json!([{"type":"tool_result","content":"ok"}]),
            ),
            user(
                "u3",
                "2026-10-01T10:20:00Z",
                serde_json::json!([{"type":"text","text":"Now test it"}]),
            ),
            reply("m2", "2026-10-01T10:21:00Z"),
        ],
    );
    let calls = collect(&[(home.clone(), None)]);
    let from = DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
        .unwrap()
        .timestamp();
    let all = report(
        &calls,
        from,
        from + 86_400,
        0,
        &HashMap::new(),
        &Prices::default(),
        &Filter::default(),
    );
    assert_eq!(all.totals.user_messages, 2);
    assert_eq!(all.totals.calls, 2);
    assert_eq!(
        all.totals.active_minutes, 10,
        "two separate five-minute slots"
    );
    assert_eq!(all.duration_seconds, 21 * 60);
    assert_eq!(all.options.projects, vec!["/repo/app".to_string()]);
    let elsewhere = Filter {
        project: Some("/other".into()),
        ..Filter::default()
    };
    assert_eq!(
        report(
            &calls,
            from,
            from + 86_400,
            0,
            &HashMap::new(),
            &Prices::default(),
            &elsewhere
        )
        .totals
        .total,
        0
    );
    let parent = Filter {
        project: Some("/repo".into()),
        model: Some("claude-sonnet-5-5".into()),
        ..Filter::default()
    };
    let narrowed = report(
        &calls,
        from,
        from + 86_400,
        0,
        &HashMap::new(),
        &Prices::default(),
        &parent,
    );
    assert_eq!(
        (narrowed.totals.calls, narrowed.totals.user_messages),
        (2, 2),
        "a project includes its subdirectories"
    );
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn a_user_price_wins_and_an_unpriced_model_gains_a_cost() {
    let mut prices = Prices::default();
    prices.models.insert(
        "gpt-6-astra".into(),
        Price {
            input: 1.0,
            output: 8.0,
            cache_read: 0.1,
            cache_write: None,
            cache_write_1h: None,
        },
    );
    let tokens = Tokens {
        input: 1_000_000,
        output: 1_000_000,
        cache_read: 1_000_000,
        ..Tokens::default()
    };
    assert_eq!(
        prices.cost_of(ProviderKind::Codex, "GPT-6-Astra", &tokens),
        Some(9.1)
    );
    assert!(
        prices
            .of("claude-opus-5-5")
            .is_some_and(|(_, custom)| !custom)
    );
    assert_eq!(
        prices.cost_of(ProviderKind::ClaudeCode, "mystery-model", &tokens),
        None
    );
}

#[test]
fn gpt_is_priced_and_only_claude_pays_extra_to_cache() {
    let prices = Prices::default();
    let writes = Tokens {
        cache_write_5m: 1_000_000,
        ..Tokens::default()
    };
    let gpt = prices.of("gpt-5.5").expect("GPT has a published price").0;
    assert_eq!(gpt.cache_write, None);
    assert_eq!(
        prices.cost_of(ProviderKind::Codex, "gpt-5.5-codex", &writes),
        Some(gpt.input),
        "a Codex cache write costs plain input"
    );
    let opus = prices.of("claude-opus-5-5").unwrap().0;
    assert_eq!(
        prices.cost_of(ProviderKind::ClaudeCode, "claude-opus-5-5", &writes),
        opus.cache_write,
        "Claude pays the listed write price"
    );
    // A price the user set without a write price keeps the old rule on Claude.
    let mut custom = Prices::default();
    custom.models.insert(
        "claude-house".into(),
        Price {
            input: 2.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: None,
            cache_write_1h: None,
        },
    );
    assert_eq!(
        custom.cost_of(ProviderKind::ClaudeCode, "claude-house", &writes),
        Some(2.5)
    );
}

#[test]
fn without_a_learned_size_the_window_is_extrapolated_and_unsure() {
    let (total, remaining, total_cost, remaining_cost, low) = estimate(25.0, 2_000_000, 10.0, None);
    assert_eq!((total, remaining), (Some(8_000_000), Some(6_000_000)));
    assert_eq!((total_cost, remaining_cost), (Some(40.0), Some(30.0)));
    assert!(low);
    assert_eq!(estimate(0.0, 10, 0.0, None).0, None);
}

#[test]
fn a_reset_time_is_read_in_seconds_milliseconds_or_rfc3339() {
    use serde_json::json;
    assert_eq!(epoch(&json!(1_791_077_410)), Some(1_791_077_410));
    // A number too large to be seconds is milliseconds.
    assert_eq!(epoch(&json!(1_791_077_410_500u64)), Some(1_791_077_410));
    assert_eq!(epoch(&json!("2026-10-04T11:00:00Z")), Some(1_791_111_600));
    assert_eq!(epoch(&json!("1791077410")), Some(1_791_077_410));
    assert_eq!(epoch(&json!("soon")), None);
    assert_eq!(epoch(&Value::Null), None);
}

#[test]
fn what_is_left_is_the_unused_share_of_the_largest_window_seen() {
    let mut history = Vec::new();
    assert_eq!(window_size(5.0, 2_000_000, 1.0), None, "too coarse");
    // Used alone: 2M at 20% shows a window of about 9.8M.
    learn(
        &mut history,
        1_000,
        window_size(19.5, 1_950_000, 9.75).unwrap(),
    );
    // The next window, shared with another device: the logs see less.
    learn(
        &mut history,
        20_000,
        window_size(39.5, 1_000_000, 5.0).unwrap(),
    );
    // The same window read again, its reset a few seconds off.
    learn(
        &mut history,
        20_030,
        window_size(49.5, 1_100_000, 5.5).unwrap(),
    );
    assert_eq!(history.len(), 2);
    let size = capacity(&history).unwrap();
    assert_eq!(size, (9_750_000, 48.75));
    // Half used, whoever used it: half the window is left.
    let (total, remaining, _, remaining_cost, low) = estimate(50.0, 300_000, 1.0, Some(size));
    assert_eq!((total, remaining), (Some(9_750_000), Some(4_875_000)));
    assert_eq!(remaining_cost, Some(24.375));
    assert!(!low);
    // A fresh window, nothing logged here yet, is all left.
    assert_eq!(estimate(0.0, 0, 0.0, Some(size)).1, Some(9_750_000));
    for at in 2..10 {
        learn(&mut history, at * 20_000, (1, 0.0));
    }
    assert_eq!(history.len(), LEARNED_WINDOWS);
}

#[test]
fn a_window_counts_only_what_was_used_since_the_plan_changed() {
    let mut learned: HashMap<String, Vec<Learned>> = HashMap::new();
    let mut plans = HashMap::new();
    // Read for the first time with nothing learned: held all along.
    assert_eq!(plan_since(&mut plans, &learned, "a", "pro", 100), 0);
    assert_eq!(plan_since(&mut plans, &learned, "a", "pro", 200), 0);
    // An upgrade starts the count at the reading that saw it, and keeps it.
    assert_eq!(plan_since(&mut plans, &learned, "a", "max", 300), 300);
    assert_eq!(plan_since(&mut plans, &learned, "a", "max", 400), 300);
    // An account first read after a change: sizes learned only under another plan.
    learned.insert("b/secondary/pro".into(), Vec::new());
    learned.insert("b/primary/".into(), Vec::new());
    assert_eq!(plan_since(&mut plans, &learned, "b", "max", 500), 500);
    // One already learned under this plan has held it.
    learned.insert("c/primary/pro".into(), Vec::new());
    learned.insert("c/primary/max".into(), Vec::new());
    assert_eq!(plan_since(&mut plans, &learned, "c", "max", 600), 0);
}

#[test]
fn prices_exist_only_for_published_models() {
    let prices = Prices::default();
    let listed = |model: &str| {
        prices
            .of(model)
            .map(|(price, _)| (price.input, price.output, price.cache_read))
    };
    assert_eq!(listed("claude-opus-5-5"), Some((4.0, 20.0, 0.20)));
    assert_eq!(listed("claude-fable-5-1"), Some((10.0, 50.0, 0.25)));
    assert_eq!(
        listed("claude-sonnet-4-5-20250929"),
        Some((3.0, 15.0, 0.30))
    );
    // Retired models LiteLLM no longer lists keep their published price.
    assert_eq!(listed("claude-opus-4-1-20250805"), Some((15.0, 75.0, 1.50)));
    assert_eq!(listed("claude-3-5-haiku-20241022"), Some((0.8, 4.0, 0.08)));
    assert_eq!(listed("gpt-5.5"), Some((5.0, 30.0, 0.5)));
    assert_eq!(listed("house-model-x"), None, "no published price");
}

#[test]
fn a_codex_account_reads_its_limits_from_the_newest_session_log() {
    let home = std::env::temp_dir().join(format!("tendly-codex-limits-{}", uuid::Uuid::new_v4()));
    let day = home.join("sessions/2026/10/04");
    std::fs::create_dir_all(&day).unwrap();
    let line = |at: &str, used: f64| {
        serde_json::json!({"timestamp":at,"type":"event_msg","payload":{"type":"token_count","info":null,
            "rate_limits":{"primary":{"used_percent":used,"window_minutes":10080,"resets_at":1791077410},"secondary":null,"plan_type":"plus"}}})
        .to_string()
    };
    std::fs::write(
        day.join("rollout.jsonl"),
        [
            line("2026-10-04T10:00:00Z", 9.0),
            line("2026-10-04T11:00:00Z", 11.0),
            "{\"type\":\"session_meta\"}".into(),
        ]
        .join("\n"),
    )
    .unwrap();
    let logged = crate::adapters::adapter(ProviderKind::Codex)
        .unwrap()
        .logged_limits(&home)
        .expect("a reading");
    let primary = logged.limits.primary.unwrap();
    assert_eq!(primary.used_percent, 11.0, "the last reading wins");
    assert_eq!(primary.window_minutes, Some(10080.0));
    assert!(logged.limits.secondary.is_none());
    assert_eq!(logged.plan.as_deref(), Some("plus"));
    assert_eq!(logged.observed_at, 1791111600);
    assert!(
        crate::adapters::adapter(ProviderKind::Codex)
            .unwrap()
            .logged_limits(&home.join("missing"))
            .is_none()
    );
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn speed_and_failures_come_from_what_each_client_logs() {
    let home = std::env::temp_dir().join(format!("tendly-usage-{}", uuid::Uuid::new_v4()));
    let claude = home.join("projects/-repo/c3.jsonl");
    let base = |kind: &str, uuid: &str, at: &str| serde_json::json!({"type":kind,"uuid":uuid,"timestamp":at,"sessionId":"c3","cwd":"/repo"});
    let mut request = base("api-request", "q1", "2026-10-01T10:00:00.000Z");
    request["id"] = "req-1".into();
    request["params"] = serde_json::json!({"model":"claude-opus-5-5"});
    let reply = |at: &str| {
        let mut line = base("assistant", "a1", at);
        line["requestId"] = "r1".into();
        line["requestRef"] = "req-1".into();
        line["message"] = serde_json::json!({"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":10,"output_tokens":200}});
        line
    };
    let mut retry = base("system", "e1", "2026-10-01T10:05:00Z");
    retry["subtype"] = "api_error".into();
    let mut failure = base("assistant", "e2", "2026-10-01T10:06:00Z");
    failure["isApiErrorMessage"] = true.into();
    failure["message"] = serde_json::json!({"id":"x","model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0}});
    // Thinking ends at two seconds, the reply at four: four seconds it took.
    write(
        &claude,
        &[
            request,
            reply("2026-10-01T10:00:02.000Z"),
            reply("2026-10-01T10:00:04.000Z"),
            retry,
            failure,
        ],
    );
    let pi = home.join("sessions/--repo--/p1.jsonl");
    let message = |id: &str, sent: i64, at: &str, stop: &str, error: Option<&str>| {
        serde_json::json!({"type":"message","id":id,"timestamp":at,"message":{"role":"assistant","model":"glm-5.3-flash",
            "timestamp":sent,"stopReason":stop,"errorMessage":error,"usage":{"input":5,"output":100}}})
    };
    let sent = DateTime::parse_from_rfc3339("2026-10-01T11:00:00Z")
        .unwrap()
        .timestamp_millis();
    write(
        &pi,
        &[
            serde_json::json!({"type":"session","id":"p1","timestamp":"2026-10-01T11:00:00Z","cwd":"/repo"}),
            message("1", sent, "2026-10-01T11:00:01.000Z", "stop", None),
            message(
                "2",
                sent,
                "2026-10-01T11:00:02Z",
                "error",
                Some("Request timed out."),
            ),
            // The person stopping a reply is not the endpoint failing.
            message(
                "3",
                sent,
                "2026-10-01T11:00:03Z",
                "error",
                Some("This operation was aborted"),
            ),
        ],
    );
    let calls = collect(&[(home.clone(), None)]);
    let from = DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
        .unwrap()
        .timestamp();
    let result = report(
        &calls,
        from,
        from + 86_400,
        0,
        &HashMap::new(),
        &Prices::default(),
        &Filter::default(),
    );
    let row = |model: &str| result.models.iter().find(|row| row.model == model).unwrap();
    let opus = &row("claude-opus-5-5").bucket;
    assert_eq!(opus.calls, 1, "a reply logged block by block is one call");
    assert_eq!((opus.timed_output, opus.timed_ms), (200, 4000));
    assert_eq!(
        (opus.answered, opus.failed, opus.retried),
        (1, 1, 1),
        "a failure is put down to the model last asked for"
    );
    let glm = &row("glm-5.3-flash").bucket;
    assert_eq!((glm.timed_output, glm.timed_ms), (100, 1000));
    assert_eq!((glm.answered, glm.failed), (1, 1));
    assert_eq!(glm.calls, 2, "a stopped reply still spent its tokens");
    std::fs::remove_dir_all(home).ok();
}
