//! [`WebLink`]: an `orr_proto::Link` whose transport lives in the browser.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use orr_proto::{Channel, Link, LinkEvent, UNRELIABLE_MTU};

#[derive(Default)]
struct Shared {
    events: VecDeque<LinkEvent>,
    up: bool,
    /// Largest unreliable payload the transport carries now (0 = none, use the reliable channel).
    max_unreliable: usize,
    oversize_to_reliable: u64,
}

/// The transport's side of a [`WebLink`]: what the browser glue calls.
#[derive(Clone)]
pub struct LinkPort(Rc<RefCell<Shared>>);

impl LinkPort {
    /// The session is up: the link reports `Connected`.
    pub fn connected(&self, max_unreliable: usize) {
        let mut s = self.0.borrow_mut();
        s.up = true;
        s.max_unreliable = max_unreliable;
        s.events.push_back(LinkEvent::Connected);
    }

    /// The session ended.
    pub fn disconnected(&self) {
        let mut s = self.0.borrow_mut();
        if s.up {
            s.up = false;
            s.events.push_back(LinkEvent::Disconnected);
        }
    }

    /// A message arrived.
    pub fn message(&self, channel: Channel, data: Vec<u8>) {
        let mut s = self.0.borrow_mut();
        if s.up {
            s.events.push_back(LinkEvent::Message { channel, data });
        }
    }

    pub fn set_max_unreliable(&self, n: usize) {
        self.0.borrow_mut().max_unreliable = n;
    }

    /// Unreliable sends that went on the reliable channel because they did not fit a datagram.
    pub fn oversize_to_reliable(&self) -> u64 {
        self.0.borrow().oversize_to_reliable
    }
}

/// One client connection as an [`orr_proto::Link`]. An unreliable message
/// that does not fit one datagram (`UNRELIABLE_MTU`, or the browser's
/// `maxDatagramSize`, 1024 bytes in Chrome) goes on the reliable channel, as
/// in the native `NetLink`.
type Transmit = Box<dyn FnMut(Channel, &[u8])>;

/// See the module docs.
pub struct WebLink {
    shared: Rc<RefCell<Shared>>,
    transmit: Transmit,
    close: Box<dyn FnMut()>,
}

impl WebLink {
    /// `transmit` hands a message to the browser transport, `close` closes it.
    pub fn new(transmit: impl FnMut(Channel, &[u8]) + 'static, close: impl FnMut() + 'static) -> (WebLink, LinkPort) {
        let shared = Rc::new(RefCell::new(Shared::default()));
        let link = WebLink { shared: shared.clone(), transmit: Box::new(transmit), close: Box::new(close) };
        (link, LinkPort(shared))
    }
}

impl Link for WebLink {
    fn send(&mut self, channel: Channel, data: &[u8]) {
        let channel = {
            let mut s = self.shared.borrow_mut();
            if !s.up {
                return;
            }
            match channel {
                Channel::Unreliable if data.len() > s.max_unreliable.min(UNRELIABLE_MTU) => {
                    s.oversize_to_reliable += 1;
                    Channel::Reliable
                }
                c => c,
            }
        };
        (self.transmit)(channel, data);
    }

    fn poll(&mut self) -> Option<LinkEvent> {
        self.shared.borrow_mut().events.pop_front()
    }

    fn close(&mut self) {
        (self.close)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_sends_and_oversize_fallback() {
        type Sent = Rc<RefCell<Vec<(Channel, usize)>>>;
        let sent: Sent = Rc::default();
        let log = sent.clone();
        let (mut link, port) = WebLink::new(move |c, d| log.borrow_mut().push((c, d.len())), || {});
        // Not up: sends are dropped, messages ignored.
        link.send(Channel::Reliable, b"x");
        port.message(Channel::Reliable, vec![1]);
        assert!(link.poll().is_none());
        assert!(sent.borrow().is_empty());

        port.connected(1000);
        assert_eq!(link.poll(), Some(LinkEvent::Connected));
        link.send(Channel::Unreliable, &[0; 900]);
        link.send(Channel::Unreliable, &[0; 1100]); // above the datagram limit
        link.send(Channel::Reliable, &[0; 5]);
        assert_eq!(*sent.borrow(), vec![(Channel::Unreliable, 900), (Channel::Reliable, 1100), (Channel::Reliable, 5)]);
        assert_eq!(port.oversize_to_reliable(), 1);

        port.message(Channel::Unreliable, vec![7, 8]);
        assert_eq!(link.poll(), Some(LinkEvent::Message { channel: Channel::Unreliable, data: vec![7, 8] }));
        port.disconnected();
        assert_eq!(link.poll(), Some(LinkEvent::Disconnected));
        port.disconnected();
        assert!(link.poll().is_none());
    }
}
