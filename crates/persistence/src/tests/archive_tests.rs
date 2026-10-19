use super::*;
use serde_json::{Value, json};
use tendly_domain::InteractionMode;

fn without_archive_timestamps(session: &Session) -> Value {
    let mut value = serde_json::to_value(session).unwrap();
    value.as_object_mut().unwrap().remove("archived_at");
    value.as_object_mut().unwrap().remove("updated_at");
    value
}

#[test]
fn archive_is_durable_reversible_idempotent_and_preserves_history_and_canvas() {
    let fixture = TempProfileDatabase::new();
    let (original, archived, event, canvas, workspace_id);
    {
        let store = Store::open(&fixture.path).unwrap();
        let workspace = store
            .create_workspace("fixture", "/fixture/project")
            .unwrap();
        workspace_id = workspace.id;
        let mut profile = ordinary_profile();
        profile.environment = [(
            "MODE".into(),
            EnvironmentValue::Literal {
                value: "keep".into(),
            },
        )]
        .into();
        store.create_endpoint_profile(&profile).unwrap();
        let session = store
            .create_session_with_profile(
                workspace.id,
                ProviderKind::Codex,
                "Archive fixture",
                Some(profile.id),
            )
            .unwrap();
        assert!(session.archived_at.is_none());
        assert_eq!(
            serde_json::to_value(
                store
                    .set_session_archived(session.id, false)
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(&session).unwrap(),
        );
        store
            .set_interaction_mode(session.id, InteractionMode::Structured)
            .unwrap();
        store
            .set_native_conversation_id(session.id, "native-thread-retained")
            .unwrap();
        store
            .set_session_result(session.id, SessionStatus::Running, Some("retained error"))
            .unwrap();
        event = store
            .append_conversation_event(
                session.id,
                json!({"type":"message","role":"assistant","text":"retained history"}),
            )
            .unwrap();
        let layout = json!({"version":1,"root":{"type":"pane","id":"bound-pane","kind":"agent_chat","metadata":{"session_id":session.id}}}).to_string();
        store
            .save_shared_canvas_layout(&layout, 0)
            .unwrap()
            .unwrap();
        canvas = store.get_shared_canvas_layout().unwrap();
        original = store.get_session(session.id).unwrap().unwrap();
        archived = store
            .set_session_archived(session.id, true)
            .unwrap()
            .unwrap();
        assert!(archived.archived_at.is_some());
        assert_eq!(archived.updated_at, archived.archived_at.unwrap());
        assert_eq!(
            without_archive_timestamps(&archived),
            without_archive_timestamps(&original)
        );
        assert_eq!(
            serde_json::to_value(
                store
                    .set_session_archived(session.id, true)
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(&archived).unwrap(),
        );
    }
    let store = Store::open(&fixture.path).unwrap();
    assert_eq!(
        serde_json::to_value(store.get_session(original.id).unwrap().unwrap()).unwrap(),
        serde_json::to_value(&archived).unwrap()
    );
    for scope in [None, Some(workspace_id)] {
        let sessions = store.list_sessions(scope).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].archived_at, archived.archived_at);
    }
    assert_eq!(store.conversation(original.id).unwrap().events, vec![event]);
    assert_eq!(store.get_shared_canvas_layout().unwrap(), canvas);
    let restored = store
        .set_session_archived(original.id, false)
        .unwrap()
        .unwrap();
    assert!(restored.archived_at.is_none());
    assert_eq!(
        without_archive_timestamps(&restored),
        without_archive_timestamps(&original)
    );
    assert_eq!(
        serde_json::to_value(
            store
                .set_session_archived(original.id, false)
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(&restored).unwrap(),
    );
    assert!(
        store
            .set_session_archived(Uuid::new_v4(), true)
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .set_session_archived(Uuid::new_v4(), false)
            .unwrap()
            .is_none()
    );
    let mut legacy = serde_json::to_value(restored).unwrap();
    legacy.as_object_mut().unwrap().remove("archived_at");
    assert!(
        serde_json::from_value::<Session>(legacy)
            .unwrap()
            .archived_at
            .is_none()
    );
    drop(store);
    assert!(
        Store::open(&fixture.path)
            .unwrap()
            .get_session(original.id)
            .unwrap()
            .unwrap()
            .archived_at
            .is_none()
    );
}

#[test]
fn archive_keeps_all_runtime_statuses_and_native_import_identity() {
    let store = Store::open(":memory:").unwrap();
    let workspace = store
        .create_workspace("fixture", "/fixture/project")
        .unwrap();
    for (index, status) in [
        SessionStatus::Stopped,
        SessionStatus::Starting,
        SessionStatus::Running,
        SessionStatus::Waiting,
        SessionStatus::Failed,
    ]
    .into_iter()
    .enumerate()
    {
        let native_id = format!("native-thread-{index}");
        let imported = store
            .import_native_session(
                workspace.id,
                ProviderKind::ClaudeCode,
                "Imported fixture",
                "fixture-source",
                &native_id,
                "/unopened/fixture-config",
            )
            .unwrap();
        store
            .set_session_result(imported.id, status, Some("preserved"))
            .unwrap();
        let original = store.get_session(imported.id).unwrap().unwrap();
        let archived = store
            .set_session_archived(imported.id, true)
            .unwrap()
            .unwrap();
        assert_eq!(
            without_archive_timestamps(&archived),
            without_archive_timestamps(&original)
        );
        assert_eq!(
            archived.native_config_dir.as_deref(),
            Some("/unopened/fixture-config")
        );
        let imported_again = store
            .import_native_session(
                workspace.id,
                ProviderKind::ClaudeCode,
                "Changed title",
                "fixture-source",
                &native_id,
                "/unopened/fixture-config",
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(imported_again).unwrap(),
            serde_json::to_value(archived).unwrap()
        );
    }
    assert_eq!(store.list_sessions(Some(workspace.id)).unwrap().len(), 5);
}

#[test]
fn version_seven_archive_migration_preserves_existing_data_and_is_reopen_safe() {
    let fixture = TempProfileDatabase::new();
    let workspace_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let timestamp = "2026-09-11T10:00:00+00:00";
    let history = json!({"seq":1,"type":"message","role":"assistant","text":"older history"});
    {
        let connection = Connection::open(&fixture.path).unwrap();
        for migration in [
            include_str!("../../../../migrations/0001_metadata.sql"),
            include_str!("../../../../migrations/0002_endpoint_models.sql"),
            include_str!("../../../../migrations/0003_native_history.sql"),
            include_str!("../../../../migrations/0004_native_profiles.sql"),
            include_str!("../../../../migrations/0005_shared_canvas.sql"),
            include_str!("../../../../migrations/0006_session_environment.sql"),
            include_str!("../../../../migrations/0007_conversations.sql"),
        ] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 7).unwrap();
        connection.execute("INSERT INTO workspaces (id,name,root_path,created_at) VALUES (?1,'fixture','/fixture/project',?2)", params![workspace_id.to_string(), timestamp]).unwrap();
        connection.execute("INSERT INTO sessions (id,workspace_id,provider,title,status,created_at,updated_at,interaction_mode,configuration_revision,provider_session_id) VALUES (?1,?2,'codex','legacy','running',?3,?3,'structured',4,'native-preserved')", params![session_id.to_string(), workspace_id.to_string(), timestamp]).unwrap();
        connection
            .execute(
                "INSERT INTO conversation_meta (session_id,next_seq) VALUES (?1,2)",
                [session_id.to_string()],
            )
            .unwrap();
        connection.execute("INSERT INTO conversation_events (session_id,seq,event_json,byte_count) VALUES (?1,1,?2,?3)", params![session_id.to_string(), history.to_string(), history.to_string().len()]).unwrap();
        assert!(
            connection
                .prepare("SELECT archived_at FROM sessions")
                .is_err()
        );
    }
    let archived;
    {
        let store = Store::open(&fixture.path).unwrap();
        assert_eq!(
            store
                .connection
                .lock()
                .unwrap()
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            22
        );
        let session = store.get_session(session_id).unwrap().unwrap();
        assert!(session.archived_at.is_none());
        assert_eq!(session.updated_at.to_rfc3339(), timestamp);
        assert!(matches!(session.status, SessionStatus::Running));
        assert_eq!(session.configuration_revision, 4);
        assert_eq!(
            session.provider_session_id.as_deref(),
            Some("native-preserved")
        );
        assert_eq!(
            store.conversation(session_id).unwrap().events,
            vec![history.clone()]
        );
        archived = store
            .set_session_archived(session_id, true)
            .unwrap()
            .unwrap();
    }
    let reopened = Store::open(&fixture.path).unwrap();
    assert_eq!(
        reopened
            .get_session(session_id)
            .unwrap()
            .unwrap()
            .archived_at,
        archived.archived_at
    );
    assert_eq!(
        reopened.conversation(session_id).unwrap().events,
        vec![history]
    );
}
