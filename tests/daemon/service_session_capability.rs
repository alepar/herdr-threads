//! Per-session capability routing (ht-5nb.3): a v1 session refuses the v2
//! operations with `Unsupported` and stays connected.
use super::*;
use crate::protocol::{
    ids::{MessageId, OperationId, SeatId, ServiceAuthorId, ThreadId},
    pagination::PageRequest,
    service::{
        EnsureManagedThread, ManagedThread, NotificationSeverity, SERVICE_SESSION_CAPABILITY,
        ServiceHistoryQuery, ServiceNotify, ServiceReceiptsQuery, ServiceSend,
    },
};
use crate::{app::SystemClock, protocol::results::CommandResult, test_support::unserved};
use std::sync::Mutex;

const INSTANCE: &str = "6f0e5c1e-7c25-4c6a-9d77-0d3c2f5f1a01";
const BOOT: &str = "6f0e5c1e-7c25-4c6a-9d77-0d3c2f5f1a02";

struct Recorder(Mutex<Vec<&'static str>>);

impl Recorder {
    fn seen(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
}

impl LocalService for Recorder {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        handle_with_output
    );
    fn service_operation(
        &self,
        operation: ServiceOperation,
        _: &ServiceConnectionAuthority,
        _: &dyn ServiceAuthorityGate,
        _: &CallBudget,
    ) -> Result<ServiceResult, ApiError> {
        let kind = match &operation {
            ServiceOperation::EnsureThread(_) => "ensure_thread",
            ServiceOperation::Notify(_) => "notify",
            ServiceOperation::Send(_) => "send",
            ServiceOperation::History(_) => "history",
            ServiceOperation::Receipts(_) => "receipts",
            _ => "other",
        };
        self.0.lock().unwrap().push(kind);
        Ok(ServiceResult::ThreadEnsured(ManagedThread {
            thread: ThreadId::new("t"),
            owner: ServiceAuthorId::new("graph"),
            archived: false,
        }))
    }
    fn handle(
        &self,
        _: Command,
        _: crate::protocol::authority::PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Err(unserved("handle"))
    }
}

fn wire(id: &str, service: ServiceRequest) -> ServiceWireRequest {
    ServiceWireRequest {
        version: PROTOCOL_VERSION,
        request_id: id.into(),
        expected_instance: INSTANCE.into(),
        service,
    }
}

fn op_send() -> ServiceOperation {
    ServiceOperation::Send(ServiceSend {
        thread: ThreadId::new("t"),
        body: "hello".into(),
        recipients: vec![SeatId::new("s1")],
        deadline_millis: None,
        operation: OperationId::new("send-1"),
    })
}
fn op_history() -> ServiceOperation {
    ServiceOperation::History(ServiceHistoryQuery {
        thread: ThreadId::new("t"),
        page: PageRequest::default(),
        initial: None,
    })
}
fn op_receipts() -> ServiceOperation {
    ServiceOperation::Receipts(ServiceReceiptsQuery {
        message: MessageId::new("m"),
        page: PageRequest::default(),
    })
}
fn op_ensure() -> ServiceOperation {
    ServiceOperation::EnsureThread(EnsureManagedThread {
        thread: ThreadId::new("t"),
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("ensure-1"),
    })
}
fn op_notify() -> ServiceOperation {
    ServiceOperation::Notify(ServiceNotify {
        thread: ThreadId::new("t"),
        severity: NotificationSeverity::Info,
        event_json: serde_json::json!({"k": 1}),
        operation: OperationId::new("notify-1"),
    })
}

struct Session {
    client: UnixStream,
    handler: Arc<Recorder>,
    gate: Arc<LiveServiceGate>,
    server: tokio::task::JoinHandle<io::Result<()>>,
    next: u32,
}

impl Session {
    async fn open(capability: &str) -> Self {
        let (client, server) = UnixStream::pair().unwrap();
        let handler = Arc::new(Recorder(Mutex::new(Vec::new())));
        let gate = Arc::new(LiveServiceGate::new());
        let first = wire(
            "register",
            ServiceRequest::Register(ServiceRegister {
                capability: capability.into(),
            }),
        );
        let server = tokio::spawn(serve_registered(
            server,
            first.clone(),
            tokio::time::Instant::now() + Duration::from_secs(10),
            INSTANCE.into(),
            BOOT.into(),
            handler.clone(),
            Arc::new(SystemClock::new()),
            gate.clone(),
            Cancellation::default(),
        ));
        let mut session = Self {
            client,
            handler,
            gate,
            server,
            next: 0,
        };
        let registered = session.read().await;
        assert!(registered.correlates_to(&first));
        assert!(matches!(
            registered.result,
            Ok(ServiceResult::Registered(_))
        ));
        session
    }

    async fn read(&mut self) -> ServiceWireResponse {
        let frame = read_frame(&mut self.client).await.unwrap();
        serde_json::from_slice(&frame).unwrap()
    }

    async fn call(
        &mut self,
        operation: ServiceOperation,
    ) -> (ServiceWireRequest, ServiceWireResponse) {
        self.next += 1;
        let request = wire(
            &format!("op-{}", self.next),
            ServiceRequest::Operation(operation),
        );
        write_frame(&mut self.client, &serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        let response = self.read().await;
        (request, response)
    }

    fn connected(&self) -> bool {
        self.gate.inspect(INSTANCE, BOOT).connected
    }

    async fn finish(self) {
        drop(self.client);
        self.server.await.unwrap().unwrap();
    }
}

fn code(response: &ServiceWireResponse) -> ErrorCode {
    response.result.as_ref().unwrap_err().code.clone()
}

#[tokio::test]
async fn v1_session_refuses_v2_operations_and_keeps_serving() {
    let mut session = Session::open(SERVICE_SESSION_CAPABILITY).await;
    for operation in [op_send(), op_history(), op_receipts()] {
        let (request, response) = session.call(operation).await;
        assert!(response.correlates_to(&request));
        assert_eq!(code(&response), ErrorCode::Unsupported);
        assert!(session.connected(), "refusal must not drop the session");
    }
    assert_eq!(session.handler.seen(), Vec::<&str>::new());
    let (request, response) = session.call(op_ensure()).await;
    assert!(response.correlates_to(&request));
    assert!(matches!(
        response.result,
        Ok(ServiceResult::ThreadEnsured(_))
    ));
    assert_eq!(session.handler.seen(), vec!["ensure_thread"]);
    assert!(session.connected());
    session.finish().await;
}

#[tokio::test]
async fn v2_session_dispatches_send_and_reads() {
    let mut session = Session::open(SERVICE_SESSION_CAPABILITY_V2).await;
    for operation in [op_send(), op_history(), op_receipts(), op_ensure()] {
        let (request, response) = session.call(operation).await;
        assert!(response.correlates_to(&request));
        assert!(response.result.is_ok(), "{:?}", response.result);
    }
    assert_eq!(
        session.handler.seen(),
        vec!["send", "history", "receipts", "ensure_thread"]
    );
    session.finish().await;
}

#[tokio::test]
async fn v1_client_flow_is_unchanged() {
    let mut session = Session::open(SERVICE_SESSION_CAPABILITY).await;
    for operation in [op_ensure(), op_notify()] {
        let (request, response) = session.call(operation).await;
        assert!(response.correlates_to(&request));
        assert!(response.result.is_ok(), "{:?}", response.result);
    }
    assert_eq!(session.handler.seen(), vec!["ensure_thread", "notify"]);
    session.finish().await;
}
