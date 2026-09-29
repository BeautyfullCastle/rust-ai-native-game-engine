use std::time::Duration;

/// Limits and timers, shared by every backend.
#[derive(Clone, Debug)]
pub struct NetConfig {
    /// Largest accepted or sent message payload in bytes. Larger incoming
    /// frames end the connection with `ProtocolViolation`.
    pub max_message_size: usize,
    /// Interval of keepalive packets (QUIC PING / WebSocket ping).
    pub keepalive_interval: Duration,
    /// A connection with no incoming traffic for this long ends with `TimedOut`.
    pub idle_timeout: Duration,
    /// Time allowed for connect and handshake.
    pub connect_timeout: Duration,
    /// Server only: connections (including handshakes in progress) above this are refused.
    pub max_connections: usize,
    /// Capacity of the event queue. When the caller stops polling and the
    /// queue is full, reading from the network pauses (flow control), except
    /// unreliable datagrams, which are dropped.
    pub event_queue: usize,
    /// Reliable bytes queued per connection above which `send` returns `Backpressure`.
    pub max_queued_send_bytes: usize,
    /// QUIC: send unreliable messages over the reliable stream when the peer has no datagram support.
    pub datagram_fallback: bool,
    /// WebSocket: an unreliable message is dropped when more than this many bytes are queued.
    pub unreliable_backlog_limit: usize,
    /// Number of tokio worker threads inside the endpoint.
    pub worker_threads: usize,
}

impl Default for NetConfig {
    fn default() -> Self {
        NetConfig {
            max_message_size: 4 * 1024 * 1024,
            keepalive_interval: Duration::from_secs(2),
            idle_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(5),
            max_connections: 1024,
            event_queue: 8192,
            max_queued_send_bytes: 16 * 1024 * 1024,
            datagram_fallback: true,
            unreliable_backlog_limit: 64 * 1024,
            worker_threads: 2,
        }
    }
}
