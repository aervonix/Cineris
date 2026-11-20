//! The fetch client behind extensions' `cineris.http.fetch`
//! (`cineris_llm::fetch`), against a resolver that answers what each test
//! says and the scripted server on the loopback interface.
//!
//! Nothing here asks the system's resolver for anything but `localhost`, or
//! leaves this machine. A test that is refused before it connects runs with
//! no async runtime at all, so a connection it should not have tried would
//! fail it, and the time limits are on the fake clock, which the test moves.
//! One test starts this binary again as a child (`sys_…`), to set the proxy
//! variables for it alone.

use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use cineris_base::CancelToken;
use cineris_llm::fetch::{DEFAULT_TIMEOUT, REQUEST_BODY_LIMIT, RESPONSE_BODY_LIMIT};
use cineris_llm::{
    AddressAllowance, AddressKind, BoxFuture, FetchError, FetchMethod, FetchRequest, FetchResponse,
    Fetcher, Refusal, Resolve, SystemResolver,
};
use cineris_testkit::role::Role;
use cineris_testkit::{FakeClock, FakeTimer, Gate, ServerReply, SseServer, block_on, join};

use crate::{START, secs};

/// Runs a test's async body on a runtime of its own: the HTTP client needs
/// one, with its I/O driver.
fn run<F: Future>(body: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(body)
}

fn ip(text: &str) -> IpAddr {
    IpAddr::from_str(text).unwrap()
}

/// The scripted server's address.
fn address_of(server: &SseServer) -> SocketAddr {
    server
        .base_url()
        .trim_start_matches("http://")
        .parse()
        .unwrap()
}

/// An allowance that names the scripted server's address.
fn the_server() -> AddressAllowance {
    AddressAllowance::none().with_address(ip("127.0.0.1"))
}

/// A resolver that answers from a script, one answer for each lookup, and
/// keeps every name it was asked. With no answer left, a lookup fails at
/// once, or never ends when the test asked for one that hangs.
#[derive(Debug, Default)]
struct Answers {
    script: Mutex<VecDeque<io::Result<Vec<IpAddr>>>>,
    asked: Mutex<Vec<String>>,
    hang: bool,
}

impl Answers {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Resolve for Answers {
    fn resolve<'a>(&'a self, host: &'a str) -> BoxFuture<'a, io::Result<Vec<IpAddr>>> {
        self.asked.lock().unwrap().push(host.to_owned());
        let answer = self.script.lock().unwrap().pop_front();
        let hang = self.hang;
        Box::pin(async move {
            match answer {
                Some(answer) => answer,
                None if hang => std::future::pending().await,
                None => Err(io::Error::other("the test's resolver has no answer left")),
            }
        })
    }
}

/// A fetcher over a scripted resolver and a fake timer the test moves.
struct Rig {
    resolver: Arc<Answers>,
    timer: FakeTimer,
    fetcher: Fetcher,
}

impl Rig {
    /// Each lookup is answered with the next list of addresses.
    fn answering<const N: usize>(answers: [&[&str]; N]) -> Self {
        Self::with(
            answers
                .into_iter()
                .map(|addresses| Ok(addresses.iter().map(|address| ip(address)).collect()))
                .collect(),
        )
    }

    /// Every lookup waits for ever.
    fn never_answering() -> Self {
        Self::scripted(VecDeque::new(), true)
    }

    fn with(script: VecDeque<io::Result<Vec<IpAddr>>>) -> Self {
        Self::scripted(script, false)
    }

    fn scripted(script: VecDeque<io::Result<Vec<IpAddr>>>, hang: bool) -> Self {
        let resolver = Arc::new(Answers {
            script: Mutex::new(script),
            asked: Mutex::default(),
            hang,
        });
        let clock = FakeClock::starting_at(START).unwrap();
        let timer = FakeTimer::manual(&clock);
        let fetcher = Fetcher::new(resolver.clone(), Arc::new(timer.clone()));
        Self {
            resolver,
            timer,
            fetcher,
        }
    }

    async fn fetch(
        &self,
        request: FetchRequest,
        allowance: &AddressAllowance,
    ) -> Result<FetchResponse, FetchError> {
        self.fetcher
            .fetch(request, allowance, DEFAULT_TIMEOUT, CancelToken::new())
            .await
    }
}

fn refused(refusal: Refusal) -> FetchError {
    FetchError::Refused(refusal)
}

#[test]
fn a_name_at_an_address_outside_the_internet_is_refused_with_its_reason() {
    use AddressKind::*;
    for (address, kind) in [
        ("127.0.0.1", Loopback),
        ("169.254.169.254", Metadata),
        ("10.0.0.1", Private),
        ("172.16.0.1", Private),
        ("192.168.1.1", Private),
        ("100.64.0.1", Shared),
        ("fd00::1", UniqueLocal),
        ("fe80::1", LinkLocal),
        ("::ffff:127.0.0.1", Loopback),
        ("::1", Loopback),
    ] {
        let rig = Rig::answering([&[address]]);
        // No runtime: a connection would fail the test.
        let error = block_on(rig.fetch(
            FetchRequest::get("https://fetch.test/data"),
            &AddressAllowance::none(),
        ))
        .unwrap_err();
        assert_eq!(
            error,
            refused(Refusal::Address {
                host: "fetch.test".to_owned(),
                address: ip(address),
                kind,
            }),
            "{address}"
        );
        let text = error.to_string();
        assert!(
            text.contains(address) && text.contains(kind.describe()),
            "{text}"
        );
        assert_eq!(rig.resolver.asked(), ["fetch.test"]);
    }
}

#[test]
fn one_address_outside_the_internet_refuses_the_name_whatever_else_it_has() {
    for found in [
        ["93.184.216.34", "10.0.0.1"],
        ["10.0.0.1", "93.184.216.34"],
        ["2606:4700::1", "10.0.0.1"],
    ] {
        let rig = Rig::answering([&found]);
        let error = block_on(rig.fetch(
            FetchRequest::get("https://fetch.test/"),
            &AddressAllowance::none(),
        ))
        .unwrap_err();
        assert_eq!(
            error,
            refused(Refusal::Address {
                host: "fetch.test".to_owned(),
                address: ip("10.0.0.1"),
                kind: AddressKind::Private,
            }),
            "{found:?}: the public address is not tried in its place"
        );
    }
}

#[test]
fn a_name_that_does_not_resolve_is_an_error_of_its_own() {
    let rig = Rig::with(VecDeque::from([
        Err(io::Error::other("no such name")),
        Ok(Vec::new()),
    ]));
    for _ in 0..2 {
        let error = block_on(rig.fetch(
            FetchRequest::get("https://fetch.test/"),
            &AddressAllowance::none(),
        ))
        .unwrap_err();
        assert!(
            matches!(&error, FetchError::Resolve { host, .. } if host == "fetch.test"),
            "{error:?}"
        );
    }
}

#[test]
fn a_literal_address_the_allowance_names_is_reached() {
    let server = SseServer::start();
    server.push(
        ServerReply::status(200)
            .header("x-scripted", "yes")
            .send("hello"),
    );
    let rig = Rig::answering([]);
    let response =
        run(rig.fetch(FetchRequest::get(server.url("/path?q=1")), &the_server())).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"hello");
    assert!(!response.cut);
    assert_eq!(response.address, address_of(&server));
    assert!(
        response
            .headers
            .contains(&("x-scripted".to_owned(), "yes".to_owned())),
        "{response:?}"
    );
    assert_eq!(server.requests()[0].path, "/path?q=1");
    assert!(
        rig.resolver.asked().is_empty(),
        "a literal is not looked up"
    );

    // Not named, the same address is refused, over either scheme.
    let not_named = block_on(rig.fetch(
        FetchRequest::get(server.url("/")),
        &AddressAllowance::none(),
    ));
    assert_eq!(
        not_named.unwrap_err(),
        refused(Refusal::PlainHttp {
            host: "127.0.0.1".to_owned()
        })
    );
    let secure = server.url("/").replace("http://", "https://");
    let not_named = block_on(rig.fetch(FetchRequest::get(secure), &AddressAllowance::none()));
    assert_eq!(
        not_named.unwrap_err(),
        refused(Refusal::Address {
            host: "127.0.0.1".to_owned(),
            address: ip("127.0.0.1"),
            kind: AddressKind::Loopback,
        })
    );
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn localhost_reaches_loopback_only_when_it_is_allowed() {
    let server = SseServer::start();
    server.push(ServerReply::status(200).send("local"));
    let url = server.url("/").replace("127.0.0.1", "localhost");

    let allowed = Rig::answering([&["127.0.0.1"]]);
    let response = run(allowed.fetch(
        FetchRequest::get(url.clone()),
        &AddressAllowance::none().with_localhost(),
    ))
    .unwrap();
    assert_eq!(response.body, b"local");
    assert_eq!(response.address, address_of(&server));
    assert_eq!(allowed.resolver.asked(), ["localhost"]);

    // Not allowed: plain `http` is refused before a lookup, and naming
    // 127.0.0.1 is not naming `localhost`.
    for allowance in [AddressAllowance::none(), the_server()] {
        let rig = Rig::answering([&["127.0.0.1"]]);
        let error = block_on(rig.fetch(FetchRequest::get(url.clone()), &allowance)).unwrap_err();
        assert_eq!(
            error,
            refused(Refusal::PlainHttp {
                host: "localhost".to_owned()
            })
        );
        assert!(rig.resolver.asked().is_empty());
    }
    // And over `https` its loopback address is refused.
    let rig = Rig::answering([&["127.0.0.1", "::1"]]);
    let error = block_on(rig.fetch(
        FetchRequest::get(url.replace("http://", "https://")),
        &AddressAllowance::none(),
    ))
    .unwrap_err();
    assert_eq!(
        error,
        refused(Refusal::Address {
            host: "localhost".to_owned(),
            address: ip("127.0.0.1"),
            kind: AddressKind::Loopback,
        })
    );
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn the_name_is_looked_up_once_and_the_connection_goes_to_the_checked_address() {
    let server = SseServer::start();
    server.push(ServerReply::status(200).send("checked"));
    // A second lookup would be told another loopback address, where
    // nothing listens.
    let rig = Rig::answering([&["127.0.0.1"], &["127.0.0.2"]]);
    let url = server.url("/").replace("127.0.0.1", "localhost");
    let response = run(rig.fetch(
        FetchRequest::get(url),
        &AddressAllowance::none().with_localhost(),
    ))
    .unwrap();
    assert_eq!(response.address, address_of(&server));
    assert_eq!(response.body, b"checked");
    assert_eq!(rig.resolver.asked(), ["localhost"], "asked exactly once");
    assert_eq!(server.requests().len(), 1);
}

/// A port that takes each connection and closes it at once, and keeps the
/// address each one reached.
struct Closer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Vec<SocketAddr>>,
}

impl Closer {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut reached = Vec::new();
            for connection in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(connection) = connection {
                    reached.push(connection.local_addr().unwrap());
                }
            }
            reached
        });
        Self {
            address,
            stop,
            thread,
        }
    }

    /// Stops, and gives the address each connection reached.
    fn finish(self) -> Vec<SocketAddr> {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the thread from its wait for a connection.
        drop(TcpStream::connect(self.address));
        self.thread.join().unwrap()
    }
}

#[test]
fn https_to_a_name_connects_to_the_checked_address_and_plain_http_is_refused() {
    let closer = Closer::start();
    let port = closer.address.port();
    let rig = Rig::answering([&["127.0.0.1"]]);

    // Plain `http` to a name is refused before it is looked up.
    let error = block_on(rig.fetch(
        FetchRequest::get(format!("http://fetch.test:{port}/")),
        &the_server(),
    ))
    .unwrap_err();
    assert_eq!(
        error,
        refused(Refusal::PlainHttp {
            host: "fetch.test".to_owned()
        })
    );
    assert!(rig.resolver.asked().is_empty());

    // Over `https` it goes to the address the lookup gave and was checked,
    // with no lookup of the client's own: the name is not one the system
    // knows. The port closes the connection, so TLS fails there.
    let error = run(rig.fetch(
        FetchRequest::get(format!("https://fetch.test:{port}/")),
        &the_server(),
    ))
    .unwrap_err();
    assert!(matches!(error, FetchError::Connect(_)), "{error:?}");
    assert_eq!(rig.resolver.asked(), ["fetch.test"]);
    assert_eq!(closer.finish(), [SocketAddr::new(ip("127.0.0.1"), port)]);
}

/// A port of five digits that closes each connection, as [`Closer`] does:
/// the TLS fixture's URLs have five digits to write a port over.
fn a_closer_of_five_digits() -> Closer {
    loop {
        let closer = Closer::start();
        if closer.address.port() >= 10_000 {
            return closer;
        }
        drop(closer.finish());
    }
}

/// The fixture's certificate with `port` written over each of its URLs'
/// `65432` (its issuer, its OCSP responder, its revocation list).
fn a_leaf_naming(port: u16) -> Vec<u8> {
    let mut leaf = include_bytes!("../../fixtures/fetch/leaf.der").to_vec();
    let written = port.to_string();
    assert_eq!(written.len(), 5);
    let mut found = 0;
    let mut at = 0;
    while let Some(offset) = leaf[at..].windows(5).position(|bytes| bytes == b"65432") {
        let start = at + offset;
        leaf[start..start + 5].copy_from_slice(written.as_bytes());
        at = start + 5;
        found += 1;
    }
    assert_eq!(found, 3, "the fixture names three URLs");
    leaf
}

/// A TLS server on loopback for one connection, which presents `leaf` and
/// reads until the client gives up on it. Its thread is not waited for: a
/// client that never connects leaves it waiting, and nothing else.
fn a_tls_server_presenting(leaf: Vec<u8>) -> SocketAddr {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let key = include_bytes!("../../fixtures/fetch/leaf-key.der").to_vec();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(leaf)],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
    )
    .unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let connection = rustls::ServerConnection::new(Arc::new(config)).unwrap();
        let mut tls = rustls::StreamOwned::new(connection, stream);
        let mut buffer = [0_u8; 1024];
        // The client refuses the certificate, and the handshake ends there.
        let _ = std::io::Read::read(&mut tls, &mut buffer);
    });
    address
}

/// What a server's certificate names is never fetched: the certificate is
/// checked against the system's roots with nothing retrieved, so a server a
/// script chose cannot send the user's machine to an address the fetch
/// refuses. The platform's verifier on Windows fetched a certificate's
/// issuer and its revocation lists from the URLs it names, to any address,
/// on the thread that polled the fetch.
#[test]
fn a_servers_certificate_sends_the_machine_nowhere() {
    let named = a_closer_of_five_digits();
    let server = a_tls_server_presenting(a_leaf_naming(named.address.port()));
    let rig = Rig::answering([&["127.0.0.1"]]);
    let error = run(rig.fetch(
        FetchRequest::get(format!("https://localhost:{}/", server.port())),
        &AddressAllowance::none().with_localhost(),
    ))
    .unwrap_err();
    assert!(matches!(error, FetchError::Connect(_)), "{error:?}");
    assert_eq!(rig.resolver.asked(), ["localhost"]);
    assert_eq!(named.finish(), Vec::<SocketAddr>::new());
}

/// This binary as a child that makes a fetch with every proxy variable set:
/// its options are the port of the fetch's target.
const PROXIED: Role = Role::new("cinfetch-proxied=");

/// The part [`crate::child_entry`] plays when a test of this module started
/// this binary in a role. Nothing otherwise.
pub(crate) fn play_a_part() {
    let Some(port) = PROXIED.options() else {
        return;
    };
    let rig = Rig::answering([&["127.0.0.1"]]);
    let error = run(rig.fetch(
        FetchRequest::get(format!("https://fetch.test:{port}/")),
        &the_server(),
    ))
    .unwrap_err();
    assert!(matches!(error, FetchError::Connect(_)), "{error:?}");
}

#[test]
fn sys_the_proxy_the_environment_names_is_not_used() {
    let (target, proxy) = (Closer::start(), Closer::start());
    let port = target.address.port();
    let proxy_url = format!("http://{}", proxy.address);
    let mut child = PROXIED.command(&port.to_string());
    for name in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        child.env(name, &proxy_url);
    }
    child.env_remove("NO_PROXY").env_remove("no_proxy");
    cineris_base::hook::started(child.get_program());
    let output = child.output().expect("run this binary as a child");
    assert!(
        output.status.success(),
        "the child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(proxy.finish().is_empty(), "nothing went to the proxy");
    assert_eq!(target.finish(), [SocketAddr::new(ip("127.0.0.1"), port)]);
}

#[test]
fn a_redirect_is_returned_and_not_followed() {
    let server = SseServer::start();
    server.push(
        ServerReply::status(302)
            .header("location", &server.url("/elsewhere"))
            .send("moved"),
    );
    server.push(ServerReply::status(200).send("followed"));
    let rig = Rig::answering([]);
    let response = run(rig.fetch(FetchRequest::get(server.url("/")), &the_server())).unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(response.body, b"moved");
    assert!(
        response
            .headers
            .contains(&("location".to_owned(), server.url("/elsewhere"))),
        "{response:?}"
    );
    assert_eq!(server.requests().len(), 1, "the redirect was not followed");
}

#[test]
fn a_refused_header_or_body_stops_the_fetch_before_anything_is_looked_up() {
    for name in ["Host", "transfer-encoding", "Proxy-Authorization", "EXPECT"] {
        // Were the header let through, the lookup's private address would
        // be the refusal instead.
        let rig = Rig::answering([&["10.0.0.1"]]);
        let request = FetchRequest {
            headers: vec![
                ("x-fine".to_owned(), "1".to_owned()),
                (name.to_owned(), "value".to_owned()),
            ],
            ..FetchRequest::get("https://fetch.test/")
        };
        let error = block_on(rig.fetch(request, &AddressAllowance::none())).unwrap_err();
        assert_eq!(
            error,
            refused(Refusal::Header {
                name: name.to_owned()
            }),
            "{name}"
        );
        assert!(rig.resolver.asked().is_empty());
    }

    let rig = Rig::answering([&["10.0.0.1"]]);
    let request = FetchRequest {
        method: FetchMethod::Post,
        body: vec![b'x'; REQUEST_BODY_LIMIT + 1],
        ..FetchRequest::get("https://fetch.test/")
    };
    let error = block_on(rig.fetch(request, &AddressAllowance::none())).unwrap_err();
    assert_eq!(
        error,
        refused(Refusal::RequestBody {
            length: REQUEST_BODY_LIMIT + 1
        })
    );
    assert!(rig.resolver.asked().is_empty());
}

/// A URL the fetch reads as another host than the one its caller checked
/// is refused before anything is looked up: the caller's patterns and
/// decision were about the host it read.
#[test]
fn a_url_read_as_another_host_than_the_one_checked_is_refused_before_any_lookup() {
    for (url, checked, read) in [
        (
            "https://evil.test\\.example.com/",
            "evil.test\\.example.com",
            "evil.test",
        ),
        ("https://0x7f.1/", "0x7f.1", "127.0.0.1"),
        ("https://ex%61mple.test/", "ex%61mple.test", "example.test"),
    ] {
        let rig = Rig::answering([&["203.0.113.7"]]);
        let request = FetchRequest {
            host: Some(checked.to_owned()),
            ..FetchRequest::get(url)
        };
        let error = block_on(rig.fetch(request, &AddressAllowance::none())).unwrap_err();
        assert_eq!(
            error,
            refused(Refusal::OtherHost {
                checked: checked.to_owned(),
                read: read.to_owned()
            }),
            "{url}"
        );
        assert!(rig.resolver.asked().is_empty(), "{url}");
    }
}

#[test]
fn a_request_body_of_the_whole_limit_is_sent() {
    let server = SseServer::start();
    server.push(ServerReply::status(201).send("stored"));
    let rig = Rig::answering([]);
    let request = FetchRequest {
        method: FetchMethod::Put,
        body: vec![b'y'; REQUEST_BODY_LIMIT],
        ..FetchRequest::get(server.url("/upload"))
    };
    let response = run(rig.fetch(request, &the_server())).unwrap();
    assert_eq!(response.status, 201);
    let received = server.requests();
    assert_eq!(received[0].body.len(), REQUEST_BODY_LIMIT);
    assert_eq!(
        received[0].header("content-length"),
        Some(REQUEST_BODY_LIMIT.to_string().as_str())
    );
}

#[test]
fn a_response_over_the_limit_is_cut_there_and_says_so() {
    let server = SseServer::start();
    server.push(
        ServerReply::status(200)
            .send(vec![b'a'; RESPONSE_BODY_LIMIT])
            .send("past the limit"),
    );
    server.push(ServerReply::status(200).send(vec![b'b'; RESPONSE_BODY_LIMIT]));
    let rig = Rig::answering([]);

    let over = run(rig.fetch(FetchRequest::get(server.url("/big")), &the_server())).unwrap();
    assert_eq!(over.body.len(), RESPONSE_BODY_LIMIT);
    assert!(over.body.iter().all(|byte| *byte == b'a'));
    assert!(over.cut, "a body that went on says it was cut");

    let exact = run(rig.fetch(FetchRequest::get(server.url("/exact")), &the_server())).unwrap();
    assert_eq!(exact.body.len(), RESPONSE_BODY_LIMIT);
    assert!(!exact.cut, "a body that ends at the limit is whole");
}

#[test]
fn each_method_reaches_the_server_as_itself() {
    let server = SseServer::start();
    for _ in FetchMethod::ALL {
        server.push(ServerReply::status(200).send("ok"));
    }
    let rig = Rig::answering([]);
    run(async {
        for method in FetchMethod::ALL {
            let request = FetchRequest {
                method,
                body: if method == FetchMethod::Get || method == FetchMethod::Head {
                    Vec::new()
                } else {
                    method.as_str().as_bytes().to_vec()
                },
                ..FetchRequest::get(server.url("/"))
            };
            let response = rig.fetch(request, &the_server()).await.unwrap();
            let expected: &[u8] = if method == FetchMethod::Head {
                b""
            } else {
                b"ok"
            };
            assert_eq!(response.body, expected, "{method:?}");
        }
    });
    let received = server.requests();
    let methods: Vec<&str> = received
        .iter()
        .map(|request| request.method.as_str())
        .collect();
    assert_eq!(methods, ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE"]);
    for request in &received[2..] {
        assert_eq!(request.body, request.method.as_bytes());
    }
}

#[test]
fn nothing_is_added_to_a_request_and_no_cookie_is_kept() {
    let server = SseServer::start();
    server.push(
        ServerReply::status(200)
            .header("set-cookie", "session=kept-nowhere; Path=/")
            .send("first"),
    );
    server.push(ServerReply::status(200).send("second"));
    let rig = Rig::answering([]);
    run(async {
        let request = FetchRequest {
            headers: vec![("x-custom".to_owned(), "mine".to_owned())],
            ..FetchRequest::get(server.url("/one"))
        };
        rig.fetch(request, &the_server()).await.unwrap();
        rig.fetch(FetchRequest::get(server.url("/two")), &the_server())
            .await
            .unwrap();
    });
    let names = |index: usize| {
        let mut names: Vec<String> = server.requests()[index]
            .headers
            .iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(0), ["accept", "host", "x-custom"]);
    assert_eq!(names(1), ["accept", "host"], "no cookie came back");
    assert_eq!(server.requests()[0].header("accept"), Some("*/*"));
    assert_eq!(server.requests()[0].header("x-custom"), Some("mine"));
}

#[test]
fn the_time_limit_ends_a_fetch_and_closes_the_connection() {
    let server = SseServer::start();
    let (sent, closed) = (Gate::new(), Gate::new());
    server.push(
        ServerReply::sse()
            .send("data: first\n\n")
            .mark(&sent)
            .hang_until_closed(&closed),
    );
    let rig = Rig::answering([]);
    run(async {
        let (result, ()) = join(
            rig.fetch(FetchRequest::get(server.url("/")), &the_server()),
            async {
                rig.timer.sleeping().await;
                assert_eq!(
                    rig.timer.running(),
                    [DEFAULT_TIMEOUT],
                    "one limit, the caller's"
                );
                sent.opened().await;
                rig.timer.advance(DEFAULT_TIMEOUT);
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), FetchError::TimedOut(DEFAULT_TIMEOUT));
        // The server sees the client hang up.
        closed.opened().await;
    });
}

#[test]
fn the_time_limit_covers_the_lookup_and_is_the_callers() {
    let rig = Rig::never_answering();
    let limit = secs(5);
    let (result, ()) = block_on(join(
        rig.fetcher.fetch(
            FetchRequest::get("https://fetch.test/"),
            &AddressAllowance::none(),
            limit,
            CancelToken::new(),
        ),
        async {
            rig.timer.sleeping().await;
            assert_eq!(rig.timer.running(), [limit]);
            rig.timer.advance(limit);
        },
    ));
    assert_eq!(result.unwrap_err(), FetchError::TimedOut(limit));
    assert_eq!(rig.resolver.asked(), ["fetch.test"]);
}

#[test]
fn cancelling_ends_a_fetch_and_closes_the_connection() {
    let server = SseServer::start();
    let (sent, closed) = (Gate::new(), Gate::new());
    server.push(
        ServerReply::sse()
            .send("data: first\n\n")
            .mark(&sent)
            .hang_until_closed(&closed),
    );
    let rig = Rig::answering([]);
    let cancel = CancelToken::new();
    run(async {
        let (result, ()) = join(
            rig.fetcher.fetch(
                FetchRequest::get(server.url("/")),
                &the_server(),
                DEFAULT_TIMEOUT,
                cancel.clone(),
            ),
            async {
                sent.opened().await;
                cancel.cancel();
                // Were the token not seen, the limit would end the fetch,
                // and say so, rather than leave the test waiting.
                rig.timer.advance(DEFAULT_TIMEOUT);
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), FetchError::Cancelled);
        closed.opened().await;
    });
}

#[test]
fn cancelling_ends_a_lookup() {
    let rig = Rig::never_answering();
    let cancel = CancelToken::new();
    let (result, ()) = block_on(join(
        rig.fetcher.fetch(
            FetchRequest::get("https://fetch.test/"),
            &AddressAllowance::none(),
            DEFAULT_TIMEOUT,
            cancel.clone(),
        ),
        async {
            cancel.cancel();
            rig.timer.advance(DEFAULT_TIMEOUT);
        },
    ));
    assert_eq!(result.unwrap_err(), FetchError::Cancelled);
}

#[test]
fn the_system_resolver_finds_localhost_off_the_async_thread() {
    let found = run(SystemResolver.resolve("localhost")).unwrap();
    assert!(!found.is_empty());
    assert!(found.iter().all(IpAddr::is_loopback), "{found:?}");
}

#[test]
fn a_fetch_can_be_sent_to_another_task() {
    fn sendable<T: Send>(_: &T) {}
    let fetcher = Fetcher::system();
    let allowance = AddressAllowance::none();
    let fetch = fetcher.fetch(
        FetchRequest::get("https://fetch.test/"),
        &allowance,
        Duration::from_secs(1),
        CancelToken::new(),
    );
    sendable(&fetch);
}
