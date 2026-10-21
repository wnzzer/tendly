//! Starting a conversation's client and supervising it until it ends.

use super::*;

/// Caller holds operations. Initialization has no prompt/model request.
pub async fn start_locked(state: &AppState, session: &Session) -> Result<Arc<ChatRuntime>> {
    let id = session.id;
    if session.provider == ProviderKind::Terminal
        || session.interaction_mode != InteractionMode::Structured
    {
        return Err(ApiError::conflict(
            "This session uses the native terminal interface",
        ));
    }
    if state
        .runtime
        .get(&id.to_string())
        .is_some_and(|r| r.running())
    {
        return Err(ApiError::conflict(
            "End the original terminal session before enabling chat; it will not be taken over",
        ));
    }
    if let Some(existing) = state.chats.get(id) {
        if existing.running() {
            existing.ready().await?;
            // A start whose request was dropped mid-way -- the page reloaded
            // while the client was coming up -- leaves a running client
            // recorded as starting. The next start or open puts that right.
            if matches!(session.status, SessionStatus::Starting) {
                db(state, move |s| {
                    s.set_session_status(id, SessionStatus::Running)
                })
                .await?;
            }
            return Ok(existing);
        }
        // Do not let an old supervisor append its exit after a replacement has
        // begun emitting the next conversation's events.
        state.chats.stop(id).await?;
    }
    if state
        .chats
        .sessions
        .lock()
        .expect("chat sessions")
        .values()
        .filter(|r| r.running())
        .count()
        >= 32
    {
        return Err(ApiError::conflict(
            "At most 32 native chat processes can run at once",
        ));
    }
    let spec = crate::launch::prepare(state, session).await?;
    let init = ChatInit {
        #[cfg(test)]
        provider: session.provider.as_str().to_owned(),
        program: spec.program.clone(),
        args: spec.args.clone(),
        cwd: spec.cwd.to_string_lossy().into_owned(),
        resume_id: session.provider_session_id.clone(),
    };
    #[cfg(test)]
    if let Some(script) = &state.chat_fixture {
        let mut command = crate::bridge::node_command(&[], script);
        command.current_dir(&spec.cwd).envs(&spec.env);
        for key in &spec.env_remove {
            command.env_remove(key);
        }
        let child = command.spawn().map_err(ApiError::internal)?;
        return run_adapter(
            state,
            session,
            crate::chat_fixture::FixtureAdapter::new(child)?,
            init,
        )
        .await;
    }
    let adapter = match session.provider {
        ProviderKind::Pi => crate::pi_chat::adapter(&spec.env, &spec.env_remove),
        ProviderKind::ClaudeCode => crate::claude_chat::adapter(&spec.env, &spec.env_remove),
        _ => crate::codex_chat::adapter(&spec.env, &spec.env_remove),
    };
    run_adapter(state, session, adapter, init).await
}

/// Record, supervise and initialize a conversation on `adapter`.
pub(crate) async fn run_adapter<A: ClientAdapter>(
    state: &AppState,
    session: &Session,
    adapter: A,
    init: ChatInit,
) -> Result<Arc<ChatRuntime>> {
    let id = session.id;
    let (commands, receiver) = mpsc::channel(8);
    let runtime = Arc::new(ChatRuntime {
        commands,
        stop: Notify::new(),
        changed: Notify::new(),
        done: Notify::new(),
        running: AtomicBool::new(true),
        finished: AtomicBool::new(false),
        ready: AtomicBool::new(false),
        busy: AtomicBool::new(false),
        approvals: Mutex::new(HashMap::new()),
        pid: Mutex::new(adapter.pid()),
    });
    db(state, move |s| {
        s.set_session_status(id, SessionStatus::Starting)
    })
    .await?;
    state
        .chats
        .sessions
        .lock()
        .expect("chat sessions")
        .insert(id, runtime.clone());
    tokio::spawn(supervise(
        state.clone(),
        id,
        runtime.clone(),
        adapter,
        receiver,
        init,
    ));
    if let Err(error) = runtime.ready().await {
        runtime.stop().await;
        db(state, move |s| {
            s.set_session_result(
                id,
                SessionStatus::Failed,
                Some("Native chat failed to initialize"),
            )
        })
        .await?;
        return Err(error);
    }
    // Every start takes the mode last chosen for this session, else the
    // preferred one. Failing to apply it is not failing to start: the chip
    // still shows the mode actually in force, and the user can change it there.
    let chosen = state.chats.chosen_permission(session.id);
    let mode = match chosen {
        Some(mode) => Some(mode),
        None => crate::preferences::initial_permission(state, session).await?,
    };
    if let Some(mode) = mode {
        let _ = runtime
            .send(json!({"type": "permission", "mode": mode}))
            .await;
    }
    db(state, move |s| {
        s.set_session_status(id, SessionStatus::Running)
    })
    .await?;
    Ok(runtime)
}

#[allow(clippy::too_many_arguments)]
async fn supervise<A: ClientAdapter>(
    state: AppState,
    id: SessionId,
    runtime: Arc<ChatRuntime>,
    mut adapter: A,
    mut commands: mpsc::Receiver<Value>,
    init: ChatInit,
) {
    let mut failure = None;
    let mut native_exit = false;
    if let Err(message) = adapter.start(&init).await {
        failure = Some(message);
    }
    *runtime.pid.lock().expect("chat pid") = adapter.pid();
    while failure.is_none() && !native_exit {
        tokio::select! {
            _=runtime.stop.notified()=>break,
            command=commands.recv()=> {
                let Some(command)=command else{break;};
                match ChatCommand::parse(&command) {
                    Ok(ChatCommand::Shutdown)=>break,
                    Ok(command)=>if let Err(message)=dispatch(&mut adapter,command).await{failure=Some(message);},
                    // The bridge answered a command it could not take with an
                    // error event and carried on; so does this.
                    Err(message)=>if handle_event(&state,id,&runtime,json!({"type":"error","message":message})).await.is_err(){failure=Some("Could not persist native conversation; stopping to avoid losing output");},
                }
            }
            event=adapter.next_event()=> {
                let event=match event {None=>break,Some(Err(message))=>{failure=Some(message);continue;},Some(Ok(event))=>event};
                let Some(event)=normalize_event(event) else {failure=Some("Native chat returned an unsupported event; update the installed bridge/client");continue;};
                if event["type"]=="exit" {native_exit=true;continue;}
                if handle_event(&state,id,&runtime,event).await.is_err(){failure=Some("Could not persist native conversation; stopping to avoid losing output");}
            }
        }
    }
    if let Some(message) = failure {
        let _ = persist(&state, id, json!({"type":"error","message":message})).await;
    }
    adapter.shutdown().await;
    runtime.running.store(false, Ordering::Release);
    runtime.ready.store(false, Ordering::Release);
    runtime.busy.store(false, Ordering::Release);
    runtime.changed.notify_waiters();
    let unresolved = runtime
        .approvals
        .lock()
        .expect("approvals")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for request_id in unresolved {
        let _ = persist(
            &state,
            id,
            json!({"type":"approval_resolved","id":request_id}),
        )
        .await;
    }
    runtime.approvals.lock().expect("approvals").clear();
    let _ = persist(&state, id, json!({"type":"exit"})).await;
    runtime.finished.store(true, Ordering::Release);
    runtime.done.notify_waiters();
    let _guard = state.operations.lock().await;
    if state
        .chats
        .get(id)
        .is_some_and(|current| Arc::ptr_eq(&current, &runtime))
    {
        let _ = db(&state, move |s| {
            if s.get_session(id)?.is_some_and(|record| {
                matches!(
                    record.status,
                    SessionStatus::Stopped | SessionStatus::Failed
                )
            }) {
                return Ok(true);
            }
            s.set_session_status(
                id,
                if failure.is_some() {
                    SessionStatus::Failed
                } else {
                    SessionStatus::Stopped
                },
            )
        })
        .await;
    }
}
