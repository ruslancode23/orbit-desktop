//! Can the window reach the web app at all?
//!
//! Variant A means the window shows a remote page. When the domain does not resolve — a beta build
//! pointed at a stand that is down, a laptop without a network — WebKit paints an empty white
//! view, which reads as "the app is broken". A cheap TCP probe before the window opens turns that
//! into a page that says what happened and offers to try again.

use std::{
    net::{TcpStream, ToSocketAddrs},
    sync::mpsc,
    time::Duration,
};

use tauri::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(1500);
const TOTAL_TIMEOUT: Duration = Duration::from_millis(3000);

/// The host and port a probe should try; `None` when the URL has nowhere to connect.
pub fn socket_address(url: &str) -> Option<(String, u16)> {
    let parsed = Url::parse(url).ok()?;
    let port = parsed.port_or_known_default()?;
    Some((parsed.host_str()?.to_string(), port))
}

/// True when something accepts a connection there.
///
/// A probe that cannot answer in time counts as unreachable: the page it leads to explains the
/// situation and can be retried, while a blank window explains nothing. The DNS lookup runs on its
/// own thread because `ToSocketAddrs` has no timeout of its own.
pub fn reachable(url: &str) -> bool {
    let Some((host, port)) = socket_address(url) else {
        return true;
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let address = (host.as_str(), port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut addresses| addresses.next());
        let connected = address
            .map(|address| TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).is_ok())
            .unwrap_or(false);
        let _ = sender.send(connected);
    });
    receiver.recv_timeout(TOTAL_TIMEOUT).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_port_the_url_implies() {
        assert_eq!(
            socket_address("https://orbit.example"),
            Some(("orbit.example".to_string(), 443))
        );
        assert_eq!(
            socket_address("http://localhost:5173"),
            Some(("localhost".to_string(), 5173))
        );
    }

    #[test]
    fn gives_up_on_something_that_is_not_a_url() {
        assert_eq!(socket_address("not a url"), None);
        assert_eq!(socket_address("mailto:hi@orbit.example"), None);
    }

    #[test]
    fn a_high_port_on_loopback_is_closed_in_these_tests() {
        // Port 9 (discard) is not served on a developer machine; if it is, the test only proves
        // that something listens, which is still a truthful answer.
        assert!(!reachable("http://127.0.0.1:9"));
    }
}
