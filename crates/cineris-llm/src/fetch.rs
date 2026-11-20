//! The fetch client: one HTTP request for an extension's
//! `cineris.http.fetch`, made so that a script reaches the internet and not
//! what only the user's machine can reach (`docs/security.md`, section 2;
//! `docs/architecture/04-providers.md`, "Transport").
//!
//! The caller decides whether a script may fetch a URL at all: it checks the
//! host against the extension's manifest and asks the permission engine.
//! This client holds the rules that only hold after the name is resolved,
//! and the limits:
//!
//! - **Schemes.** `https`, or plain `http` only to an address the caller's
//!   [`AddressAllowance`] names: a literal IP address it lists, or the name
//!   `localhost` when it allows loopback.
//! - **Addresses.** A name is resolved once, through a [`Resolve`], and every
//!   address it gives is checked. A loopback, unspecified, link-local,
//!   private, shared (carrier-grade NAT), unique-local, multicast, broadcast
//!   or cloud-metadata address, or an IPv6 form of one of those IPv4
//!   addresses, is refused unless the allowance names it ([`classify`]). One
//!   refused address refuses the fetch: no other address of the name is
//!   tried in its place.
//! - **The connection goes to a checked address, and nothing looks the name
//!   up again.** The HTTP client is made for the one request, with a
//!   resolver of its own that answers the request's host with the checked
//!   addresses and refuses every other name. Nothing under it can reach the
//!   system's resolver: `ClientBuilder::resolve_to_addrs` would override the
//!   one name and keep the system's resolver for every other, and a shared
//!   client would pool connections, so that one request could be sent on a
//!   connection that was checked for another request and another allowance.
//!   Before the response is returned, its connection's peer is checked to be
//!   one of the checked addresses ([`Refusal::UncheckedPeer`]).
//! - **No proxy.** A proxy would resolve the name itself, after the check
//!   and out of its sight. Neither the proxy environment variables nor the
//!   system's settings are read.
//! - **No redirect is followed.** A 3xx is a response, returned as it is: a
//!   redirect to another host would skip the caller's check of the host.
//! - **Headers.** The caller may not set a header HTTP itself sets (`Host`,
//!   `Connection`, `Keep-Alive`, `Transfer-Encoding`, `TE`, `Trailer`,
//!   `Upgrade`, `Content-Length`, `Expect`) or any `Proxy-` header. No
//!   cookie is kept between fetches, and nothing is added to the request
//!   beyond what HTTP needs: no credential, no provider header, no
//!   `user-agent`. The one header the HTTP client adds when the caller sets
//!   none is `accept: */*`.
//! - **Limits.** A request body of at most [`REQUEST_BODY_LIMIT`] (refused
//!   above it), a response body read to [`RESPONSE_BODY_LIMIT`] (cut there,
//!   and [`FetchResponse::cut`] says so), and a total time the caller gives,
//!   [`DEFAULT_TIMEOUT`] unless it has a reason for another, from the start
//!   of the lookup to the end of the body. The time is measured on the
//!   crate's [`Timer`], so a test moves a fake clock and waits for nothing.
//! - **Cancellation.** The caller's [`CancelToken`] ends the fetch wherever
//!   it is, and the connection is closed.
//!
//! **What a client for each request costs.** Creating it loads the system's
//! certificate store on Linux, which the provider transport measured at 1.06
//! to 1.13 ms warm (`docs/performance-budgets.md`). It is created on the
//! blocking pool, as the lookup is, so the async thread does not wait for
//! either. A fetch needs a Tokio runtime, as the provider transport does.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use cineris_base::CancelToken;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::time::{Raced, Timer, race};
use crate::transport::{BoxFuture, describe};

/// The largest request body a fetch sends: 1 MiB. A larger one is refused
/// before anything is looked up.
pub const REQUEST_BODY_LIMIT: usize = 1024 * 1024;

/// The most of a response body a fetch reads: 4 MiB. The rest is not read,
/// and the response says it was cut.
pub const RESPONSE_BODY_LIMIT: usize = 4 * 1024 * 1024;

/// The time a fetch is given when the caller has no reason to give another:
/// 30 s, from the start of the lookup to the end of the body.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// The headers HTTP sets itself, which a caller may not set. Any header
/// whose name begins `Proxy-` is refused as well.
const REFUSED_HEADERS: [&str; 9] = [
    "host",
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "content-length",
    "expect",
];

/// The cloud metadata service's address on IPv4: AWS, Azure, GCP and others.
const METADATA_V4: Ipv4Addr = Ipv4Addr::new(169, 254, 169, 254);

/// The metadata service's address on IPv6, on AWS: `fd00:ec2::254`.
const METADATA_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254);

/// The method of a fetch: the six a script may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FetchMethod {
    /// `GET`.
    Get,
    /// `HEAD`.
    Head,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// `PATCH`.
    Patch,
    /// `DELETE`.
    Delete,
}

impl FetchMethod {
    /// Every method, in the order above.
    pub const ALL: [Self; 6] = [
        Self::Get,
        Self::Head,
        Self::Post,
        Self::Put,
        Self::Patch,
        Self::Delete,
    ];

    /// The method as HTTP writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    /// The method a script named, in any case: `post` is `POST`, as a
    /// browser's `fetch` reads it. Any other method is refused.
    pub fn parse(name: &str) -> Result<Self, Refusal> {
        Self::ALL
            .into_iter()
            .find(|method| method.as_str().eq_ignore_ascii_case(name))
            .ok_or_else(|| Refusal::Method {
                name: name.to_owned(),
            })
    }

    /// Whether a request of this method says how long its body is even when
    /// the body is empty (`content-length: 0`).
    const fn sends_a_body(self) -> bool {
        matches!(self, Self::Post | Self::Put | Self::Patch)
    }

    fn to_http(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Head => reqwest::Method::HEAD,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
            Self::Delete => reqwest::Method::DELETE,
        }
    }
}

/// One fetch, as a script asked for it.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchRequest {
    /// The method.
    pub method: FetchMethod,
    /// The URL: `https`, or `http` to an address the allowance names.
    pub url: String,
    /// The headers to send, in order. A name may come more than once.
    pub headers: Vec<(String, String)>,
    /// The body: at most [`REQUEST_BODY_LIMIT`] bytes.
    pub body: Vec<u8>,
}

impl FetchRequest {
    /// A `GET` of a URL, with no headers and no body.
    #[must_use]
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: FetchMethod::Get,
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

/// Header values are left out: a script's request may carry its own
/// credential, and the URL may carry one in its query, so neither is
/// printed.
impl std::fmt::Debug for FetchRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("FetchRequest")
            .field("method", &self.method)
            .field("headers", &names)
            .field("body_bytes", &self.body.len())
            .finish_non_exhaustive()
    }
}

/// The response to a fetch, read whole or to [`RESPONSE_BODY_LIMIT`].
#[derive(Clone, PartialEq, Eq)]
pub struct FetchResponse {
    /// The status code. A 3xx is here as it came: no redirect is followed.
    pub status: u16,
    /// The response headers, in order, names in lower case. A value that is
    /// not UTF-8 has its other bytes replaced.
    pub headers: Vec<(String, String)>,
    /// The body, or its first [`RESPONSE_BODY_LIMIT`] bytes.
    pub body: Vec<u8>,
    /// Whether the body went on past [`RESPONSE_BODY_LIMIT`] and was cut
    /// there.
    pub cut: bool,
    /// The address the response came from: one that was checked.
    pub address: SocketAddr,
}

impl std::fmt::Debug for FetchResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FetchResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body_bytes", &self.body.len())
            .field("cut", &self.cut)
            .field("address", &self.address)
            .finish()
    }
}

/// The addresses outside the internet that a fetch may reach: the literal
/// IP addresses the caller's policy names, and loopback when it names
/// `localhost`.
///
/// This is a list of literal allowances, not of host patterns. A wildcard
/// in an extension's `net` list allows no address here:
/// [`add_host`](Self::add_host) takes only a literal address or `localhost`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddressAllowance {
    addresses: Vec<IpAddr>,
    localhost: bool,
}

impl AddressAllowance {
    /// Nothing outside the internet.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// This one address too.
    #[must_use]
    pub fn with_address(mut self, address: IpAddr) -> Self {
        self.addresses.push(address);
        self
    }

    /// Loopback too: the name `localhost`, and every loopback address.
    #[must_use]
    pub fn with_localhost(mut self) -> Self {
        self.localhost = true;
        self
    }

    /// Allows the host of one of the caller's literal allowances: an IPv4
    /// address, an IPv6 address with or without its brackets, or
    /// `localhost` in any case. Returns whether it was one; anything else (a
    /// name, a pattern) allows nothing and returns `false`. The caller takes
    /// any port off first.
    pub fn add_host(&mut self, host: &str) -> bool {
        if host.eq_ignore_ascii_case("localhost") {
            self.localhost = true;
            return true;
        }
        let bare = host
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(host);
        match bare.parse::<IpAddr>() {
            Ok(address) => {
                self.addresses.push(address);
                true
            }
            Err(_) => false,
        }
    }

    /// The literal addresses named.
    #[must_use]
    pub fn addresses(&self) -> &[IpAddr] {
        &self.addresses
    }

    /// Whether `localhost` is named.
    #[must_use]
    pub fn allows_localhost(&self) -> bool {
        self.localhost
    }

    /// Whether `address` is named: listed (an IPv4 address and its
    /// IPv4-mapped IPv6 form are one address here), or a loopback address
    /// when `localhost` is named.
    #[must_use]
    pub fn allows(&self, address: IpAddr) -> bool {
        let address = address.to_canonical();
        self.addresses
            .iter()
            .any(|named| named.to_canonical() == address)
            || (self.localhost && classify(address) == Some(AddressKind::Loopback))
    }
}

/// What kind of address outside the internet an address is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressKind {
    /// This machine: `127.0.0.0/8`, `::1`.
    Loopback,
    /// No address: `0.0.0.0/8`, `::`. Connecting to it reaches this
    /// machine on some systems.
    Unspecified,
    /// The local link: `169.254.0.0/16`, `fe80::/10`.
    LinkLocal,
    /// A private network: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`,
    /// and IPv6's deprecated site-local `fec0::/10`.
    Private,
    /// A carrier's shared space (carrier-grade NAT): `100.64.0.0/10`.
    Shared,
    /// IPv6's private networks: `fc00::/7`.
    UniqueLocal,
    /// A group: `224.0.0.0/4`, `ff00::/8`.
    Multicast,
    /// The local network's broadcast: `255.255.255.255`.
    Broadcast,
    /// A cloud's metadata service, which hands out the machine's
    /// credentials: `169.254.169.254`, `fd00:ec2::254`.
    Metadata,
}

impl AddressKind {
    /// What the kind is, in words a script's author and a user can read.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Loopback => "a loopback address (this machine)",
            Self::Unspecified => "an unspecified address (this machine, on some systems)",
            Self::LinkLocal => "a link-local address",
            Self::Private => "a private network's address",
            Self::Shared => "a shared address (carrier-grade NAT)",
            Self::UniqueLocal => "a unique local address (a private IPv6 network)",
            Self::Multicast => "a multicast address",
            Self::Broadcast => "the broadcast address",
            Self::Metadata => "a cloud metadata service's address",
        }
    }
}

impl std::fmt::Display for AddressKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.describe())
    }
}

/// The kind of `address` when it is outside the internet, and `None` for an
/// address of the internet.
///
/// An IPv6 address that stands for an IPv4 one is the kind of that IPv4
/// address: the IPv4-mapped form (`::ffff:a.b.c.d`), the translated form
/// (`::ffff:0:a.b.c.d`), the deprecated IPv4-compatible form (`::a.b.c.d`)
/// and NAT64's well-known prefix (`64:ff9b::a.b.c.d`), which a network with
/// DNS64 answers names with.
#[must_use]
pub fn classify(address: IpAddr) -> Option<AddressKind> {
    match address {
        IpAddr::V4(address) => classify_v4(address),
        IpAddr::V6(address) => classify_v6(address),
    }
}

fn classify_v4(address: Ipv4Addr) -> Option<AddressKind> {
    if address == METADATA_V4 {
        return Some(AddressKind::Metadata);
    }
    if address == Ipv4Addr::BROADCAST {
        return Some(AddressKind::Broadcast);
    }
    let [first, second, _, _] = address.octets();
    match (first, second) {
        (0, _) => Some(AddressKind::Unspecified),
        (127, _) => Some(AddressKind::Loopback),
        (10, _) | (172, 16..=31) | (192, 168) => Some(AddressKind::Private),
        (100, 64..=127) => Some(AddressKind::Shared),
        (169, 254) => Some(AddressKind::LinkLocal),
        (224..=239, _) => Some(AddressKind::Multicast),
        _ => None,
    }
}

fn classify_v6(address: Ipv6Addr) -> Option<AddressKind> {
    if address == METADATA_V6 {
        return Some(AddressKind::Metadata);
    }
    if address.is_unspecified() {
        return Some(AddressKind::Unspecified);
    }
    if address.is_loopback() {
        return Some(AddressKind::Loopback);
    }
    if let Some(inner) = embedded_v4(address) {
        return classify_v4(inner);
    }
    let first = address.segments()[0];
    if first & 0xffc0 == 0xfe80 {
        Some(AddressKind::LinkLocal)
    } else if first & 0xffc0 == 0xfec0 {
        Some(AddressKind::Private)
    } else if first & 0xfe00 == 0xfc00 {
        Some(AddressKind::UniqueLocal)
    } else if first & 0xff00 == 0xff00 {
        Some(AddressKind::Multicast)
    } else {
        None
    }
}

/// The IPv4 address an IPv6 address stands for, in the forms
/// [`classify`] names. `::` and `::1` are IPv6's own and are taken first.
fn embedded_v4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let [_, _, _, _, _, _, _, _, _, _, _, _, a, b, c, d] = address.octets();
    let inner = Ipv4Addr::new(a, b, c, d);
    match address.segments() {
        // IPv4-mapped, then translated, then IPv4-compatible.
        [0, 0, 0, 0, 0, 0xffff, _, _]
        | [0, 0, 0, 0, 0xffff, 0, _, _]
        | [0, 0, 0, 0, 0, 0, _, _] => Some(inner),
        // NAT64's well-known prefix.
        [0x64, 0xff9b, 0, 0, 0, 0, _, _] => Some(inner),
        _ => None,
    }
}

/// Why a fetch was refused before or without being sent: something the
/// script asked for that the rules do not allow. Each says what was
/// refused and why, in words a script's author and a user can read, and
/// none shows a header's value or the URL's query.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// The URL could not be read.
    #[error("the URL is not one a fetch can take: {detail}")]
    Url {
        /// What is wrong with it.
        detail: String,
    },
    /// A scheme other than `https` and `http`.
    #[error(
        "the scheme `{scheme}` is not one a fetch may use: only `https`, or `http` to an \
         address allowed by name"
    )]
    Scheme {
        /// The scheme.
        scheme: String,
    },
    /// Plain `http` to a host that is not an allowed address.
    #[error(
        "plain `http` goes only to an address allowed by name (a literal IP address, or \
         `localhost` when loopback is allowed), and `{host}` is not one: use `https`"
    )]
    PlainHttp {
        /// The URL's host.
        host: String,
    },
    /// A user name or password in the URL.
    #[error("a URL may not carry a user name or password: send a credential in a header")]
    Credentials,
    /// An address outside the internet that the allowance does not name.
    #[error(
        "`{host}` is at {address}, {kind}, which a fetch reaches only when that address is \
         allowed by name"
    )]
    Address {
        /// The URL's host.
        host: String,
        /// The address that was refused.
        address: IpAddr,
        /// What kind of address it is.
        kind: AddressKind,
    },
    /// A header that HTTP sets itself, or a proxy's.
    #[error(
        "the header `{name}` may not be set by a fetch: HTTP sets it itself, or it is for a proxy"
    )]
    Header {
        /// The header's name, as the request gave it.
        name: String,
    },
    /// A header whose name or value HTTP cannot carry.
    #[error("the header `{name}` cannot be sent: its name or value is not valid in HTTP")]
    InvalidHeader {
        /// The header's name, as the request gave it.
        name: String,
    },
    /// A request body over [`REQUEST_BODY_LIMIT`].
    #[error("the request body is {length} bytes, over the limit of {REQUEST_BODY_LIMIT} (1 MiB)")]
    RequestBody {
        /// Its length.
        length: usize,
    },
    /// A method other than the six.
    #[error("`{name}` is not a method a fetch may use: GET, HEAD, POST, PUT, PATCH or DELETE")]
    Method {
        /// The method, as the script named it.
        name: String,
    },
    /// The response came over a connection to an address that was not
    /// checked. The resolver the client is given makes this impossible;
    /// this is the second guard, and the response is not returned.
    #[error(
        "the connection went to {}, which is not an address that was checked",
        describe_peer(.peer)
    )]
    UncheckedPeer {
        /// The connection's peer, when the client said.
        peer: Option<SocketAddr>,
    },
}

fn describe_peer(peer: &Option<SocketAddr>) -> String {
    peer.map_or_else(
        || "an address the client did not give".to_owned(),
        |peer| peer.to_string(),
    )
}

/// Why a fetch did not return a response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The rules refused it ([`Refusal`]).
    #[error("the fetch was refused: {0}")]
    Refused(#[from] Refusal),
    /// The name could not be resolved, or has no address.
    #[error("the name `{host}` could not be resolved: {detail}")]
    Resolve {
        /// The name.
        host: String,
        /// What the resolver said.
        detail: String,
    },
    /// No connection could be made, or its TLS handshake failed.
    #[error("{0}")]
    Connect(String),
    /// The request or the response failed after the connection was made.
    #[error("{0}")]
    Failed(String),
    /// The time the caller gave ran out.
    #[error("the fetch took longer than its limit of {} s", .0.as_secs_f64())]
    TimedOut(Duration),
    /// The caller's token was cancelled.
    #[error("the fetch was cancelled")]
    Cancelled,
}

/// Looks a name up: the seam between the fetch client and the system's
/// resolver, so that a test answers with the addresses it chooses.
pub trait Resolve: Send + Sync + std::fmt::Debug {
    /// The addresses of `host`, a name (never a literal address). Asked
    /// once for each fetch to a name. The real one blocks, so it runs off
    /// the async thread.
    fn resolve<'a>(&'a self, host: &'a str) -> BoxFuture<'a, io::Result<Vec<IpAddr>>>;
}

/// The system's resolver (`getaddrinfo` and its Windows counterpart), on
/// the runtime's blocking pool.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> BoxFuture<'a, io::Result<Vec<IpAddr>>> {
        let host = host.to_owned();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                use std::net::ToSocketAddrs as _;
                (host.as_str(), 0)
                    .to_socket_addrs()
                    .map(|found| found.map(|address| address.ip()).collect())
            })
            .await
            .map_err(io::Error::other)?
        })
    }
}

/// Makes fetches. Cheap to clone; it holds no connection between fetches.
#[derive(Debug, Clone)]
pub struct Fetcher {
    resolver: Arc<dyn Resolve>,
    timer: Arc<dyn Timer>,
}

impl Fetcher {
    /// A fetcher that looks names up with `resolver` and measures a fetch's
    /// time on `timer`.
    #[must_use]
    pub fn new(resolver: Arc<dyn Resolve>, timer: Arc<dyn Timer>) -> Self {
        Self { resolver, timer }
    }

    /// The fetcher the harness runs with: the system's resolver and the
    /// runtime's timer.
    #[must_use]
    pub fn system() -> Self {
        Self::new(Arc::new(SystemResolver), Arc::new(crate::time::TokioTimer))
    }

    /// Makes one fetch, under the rules of this module.
    ///
    /// What the request itself breaks (its body's size, a header, the URL's
    /// scheme or host) is refused before anything is looked up. Then the
    /// name is resolved, its addresses checked against `allowance`, and the
    /// request sent to one of them. `timeout` runs from the lookup to the
    /// end of the body, and cancelling `cancel` ends the fetch; either
    /// closes the connection.
    pub async fn fetch(
        &self,
        request: FetchRequest,
        allowance: &AddressAllowance,
        timeout: Duration,
        cancel: CancelToken,
    ) -> Result<FetchResponse, FetchError> {
        if request.body.len() > REQUEST_BODY_LIMIT {
            return Err(Refusal::RequestBody {
                length: request.body.len(),
            }
            .into());
        }
        let headers = header_map(&request.headers)?;
        let target = Target::parse(&request.url, allowance)?;

        let mut limit = self.timer.sleep(timeout);
        let sending = self.send(request.method, &target, headers, request.body, allowance);
        match race(sending, &cancel, &mut limit).await {
            Raced::Done(result) => result,
            Raced::Cancelled => Err(FetchError::Cancelled),
            Raced::TimedOut => Err(FetchError::TimedOut(timeout)),
        }
    }

    async fn send(
        &self,
        method: FetchMethod,
        target: &Target,
        headers: HeaderMap,
        body: Vec<u8>,
        allowance: &AddressAllowance,
    ) -> Result<FetchResponse, FetchError> {
        let found = self.addresses_of(target).await?;
        let checked = check_addresses(target, &found, allowance)?;
        let client = client_for(target, &checked).await?;

        let mut sending = client
            .request(method.to_http(), target.url.clone())
            .headers(headers);
        if !body.is_empty() || method.sends_a_body() {
            sending = sending.body(body);
        }
        let mut response = sending.send().await.map_err(|error| failure(&error))?;
        let address = check_peer(response.remote_addr(), &checked)?;

        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let (body, cut) = read_body(&mut response).await?;
        Ok(FetchResponse {
            status,
            headers,
            body,
            cut,
            address,
        })
    }

    /// The addresses to check: a literal address is itself, and a name is
    /// asked of the resolver, once.
    async fn addresses_of(&self, target: &Target) -> Result<Vec<IpAddr>, FetchError> {
        let name = match &target.host {
            Host::Address(address) => return Ok(vec![*address]),
            Host::Name(name) => name,
        };
        let found = self
            .resolver
            .resolve(name)
            .await
            .map_err(|error| FetchError::Resolve {
                host: name.clone(),
                detail: error.to_string(),
            })?;
        if found.is_empty() {
            return Err(FetchError::Resolve {
                host: name.clone(),
                detail: "it has no address".to_owned(),
            });
        }
        Ok(found)
    }
}

/// The URL of a fetch, read and checked.
#[derive(Debug)]
struct Target {
    url: reqwest::Url,
    host: Host,
    port: u16,
    https: bool,
}

#[derive(Debug)]
enum Host {
    Name(String),
    Address(IpAddr),
}

impl Target {
    /// Reads `url` and checks what can be checked before a lookup: the
    /// scheme, the host's form, and that plain `http` goes to an allowed
    /// address.
    fn parse(url: &str, allowance: &AddressAllowance) -> Result<Self, Refusal> {
        if let Some(host) = crate::http::host_outside_ascii(url) {
            return Err(Refusal::Url {
                detail: crate::http::host_outside_ascii_hint(host),
            });
        }
        let url = reqwest::Url::parse(url).map_err(|error| Refusal::Url {
            detail: error.to_string(),
        })?;
        let https = match url.scheme() {
            "https" => true,
            "http" => false,
            other => {
                return Err(Refusal::Scheme {
                    scheme: other.to_owned(),
                });
            }
        };
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Refusal::Credentials);
        }
        // The URL library has already read every form of an IPv4 address
        // (`127.1`, `0x7f000001`) into the dotted one, and an IPv6 address
        // is in brackets: what is left is a name.
        let Some(text) = url.host_str().filter(|text| !text.is_empty()) else {
            return Err(Refusal::Url {
                detail: "it has no host".to_owned(),
            });
        };
        let host = if let Some(inner) = text
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
        {
            let address = inner.parse::<Ipv6Addr>().map_err(|_| Refusal::Url {
                detail: "its host is not an IPv6 address".to_owned(),
            })?;
            Host::Address(IpAddr::V6(address))
        } else if let Ok(address) = text.parse::<Ipv4Addr>() {
            Host::Address(IpAddr::V4(address))
        } else {
            Host::Name(text.to_owned())
        };
        let Some(port) = url.port_or_known_default() else {
            return Err(Refusal::Url {
                detail: "it has no port".to_owned(),
            });
        };
        let target = Self {
            url,
            host,
            port,
            https,
        };
        if !https {
            let named = match &target.host {
                Host::Name(name) => name == "localhost" && allowance.allows_localhost(),
                Host::Address(address) => allowance.allows(*address),
            };
            if !named {
                return Err(Refusal::PlainHttp {
                    host: target.host_text(),
                });
            }
        }
        Ok(target)
    }

    /// The host as the URL has it, without brackets.
    fn host_text(&self) -> String {
        match &self.host {
            Host::Name(name) => name.clone(),
            Host::Address(address) => address.to_string(),
        }
    }
}

/// Checks every address a host has, and gives the ones to connect to, with
/// the URL's port. One refused address refuses the fetch.
fn check_addresses(
    target: &Target,
    found: &[IpAddr],
    allowance: &AddressAllowance,
) -> Result<Vec<SocketAddr>, Refusal> {
    let mut checked: Vec<SocketAddr> = Vec::with_capacity(found.len());
    for &address in found {
        if let Some(kind) = classify(address)
            && !allowance.allows(address)
        {
            return Err(Refusal::Address {
                host: target.host_text(),
                address,
                kind,
            });
        }
        // `localhost` over plain `http` reaches loopback, and only that.
        if !target.https && !allowance.allows(address) {
            return Err(Refusal::PlainHttp {
                host: target.host_text(),
            });
        }
        let socket = SocketAddr::new(address, target.port);
        if !checked.contains(&socket) {
            checked.push(socket);
        }
    }
    Ok(checked)
}

/// The connection's peer, when it is one of the checked addresses.
fn check_peer(peer: Option<SocketAddr>, checked: &[SocketAddr]) -> Result<SocketAddr, Refusal> {
    match peer {
        Some(found)
            if checked.iter().any(|address| {
                address.ip().to_canonical() == found.ip().to_canonical()
                    && address.port() == found.port()
            }) =>
        {
            Ok(found)
        }
        _ => Err(Refusal::UncheckedPeer { peer }),
    }
}

/// The request's headers, each checked: none HTTP sets itself, none for a
/// proxy, and each one HTTP can carry.
fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, Refusal> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        if is_refused_header(name) {
            return Err(Refusal::Header { name: name.clone() });
        }
        let invalid = || Refusal::InvalidHeader {
            name: name.escape_debug().to_string(),
        };
        let header = HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid())?;
        let value = HeaderValue::from_str(value).map_err(|_| invalid())?;
        map.append(header, value);
    }
    Ok(map)
}

fn is_refused_header(name: &str) -> bool {
    REFUSED_HEADERS
        .iter()
        .any(|refused| name.eq_ignore_ascii_case(refused))
        || name
            .get(..6)
            .is_some_and(|start| start.eq_ignore_ascii_case("proxy-"))
}

/// The resolver the HTTP client of one fetch is given: the request's host
/// is the checked addresses, and every other name is refused, so nothing
/// under the client looks a name up.
#[derive(Debug)]
struct Pinned {
    host: String,
    addresses: Vec<SocketAddr>,
}

impl reqwest::dns::Resolve for Pinned {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let answer: Result<reqwest::dns::Addrs, Box<dyn std::error::Error + Send + Sync>> =
            if name.as_str().eq_ignore_ascii_case(&self.host) {
                Ok(Box::new(self.addresses.clone().into_iter()))
            } else {
                Err(format!(
                    "`{}` was not checked, and a fetch looks up no name but its own",
                    name.as_str()
                )
                .into())
            };
        Box::pin(std::future::ready(answer))
    }
}

/// An HTTP client for one fetch, made on the blocking pool: creating one
/// loads the system's certificate store.
async fn client_for(
    target: &Target,
    checked: &[SocketAddr],
) -> Result<reqwest::Client, FetchError> {
    let pinned = Pinned {
        host: target.host_text(),
        addresses: checked.to_vec(),
    };
    tokio::task::spawn_blocking(move || build_client(pinned))
        .await
        .map_err(|error| {
            FetchError::Failed(format!("the HTTP client could not be made: {error}"))
        })?
}

fn build_client(pinned: Pinned) -> Result<reqwest::Client, FetchError> {
    crate::transport::install_tls_provider();
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .referer(false)
        .dns_resolver(Arc::new(pinned))
        .build()
        .map_err(|error| {
            FetchError::Failed(format!(
                "the HTTP client could not be made: {}",
                describe(&error)
            ))
        })
}

/// Reads the body to its end or to [`RESPONSE_BODY_LIMIT`], and says
/// whether it was cut.
async fn read_body(response: &mut reqwest::Response) -> Result<(Vec<u8>, bool), FetchError> {
    let expected = response
        .content_length()
        .map_or(0, |length| usize::try_from(length).unwrap_or(usize::MAX))
        .min(RESPONSE_BODY_LIMIT);
    let mut body = Vec::with_capacity(expected);
    while let Some(chunk) = response.chunk().await.map_err(|error| failure(&error))? {
        let room = RESPONSE_BODY_LIMIT.saturating_sub(body.len());
        if chunk.len() > room {
            body.extend_from_slice(&chunk[..room]);
            return Ok((body, true));
        }
        body.extend_from_slice(&chunk);
    }
    Ok((body, false))
}

/// A client error, by its type: no connection (or no TLS session) is
/// [`FetchError::Connect`], anything after it [`FetchError::Failed`]. The
/// text has no URL.
fn failure(error: &reqwest::Error) -> FetchError {
    if error.is_connect() {
        FetchError::Connect(describe(error))
    } else {
        FetchError::Failed(describe(error))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;

    fn ip(text: &str) -> IpAddr {
        IpAddr::from_str(text).unwrap()
    }

    #[test]
    fn every_range_outside_the_internet_is_named() {
        use AddressKind::*;
        for (address, kind) in [
            ("127.0.0.1", Loopback),
            ("127.255.255.254", Loopback),
            ("::1", Loopback),
            ("0.0.0.0", Unspecified),
            ("0.1.2.3", Unspecified),
            ("::", Unspecified),
            ("169.254.0.1", LinkLocal),
            ("169.254.255.255", LinkLocal),
            ("fe80::1", LinkLocal),
            ("febf::1", LinkLocal),
            ("10.0.0.1", Private),
            ("10.255.255.255", Private),
            ("172.16.0.1", Private),
            ("172.31.255.255", Private),
            ("192.168.1.1", Private),
            ("192.168.255.255", Private),
            ("fec0::1", Private),
            ("100.64.0.1", Shared),
            ("100.127.255.255", Shared),
            ("fc00::1", UniqueLocal),
            ("fd00::1", UniqueLocal),
            ("fdff:ffff::1", UniqueLocal),
            ("224.0.0.1", Multicast),
            ("239.255.255.255", Multicast),
            ("ff02::1", Multicast),
            ("ff0e::1", Multicast),
            ("255.255.255.255", Broadcast),
            ("169.254.169.254", Metadata),
            ("fd00:ec2::254", Metadata),
        ] {
            assert_eq!(classify(ip(address)), Some(kind), "{address}");
        }
    }

    #[test]
    fn the_internet_is_not_named_even_at_the_edge_of_a_range() {
        for address in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "1.0.0.0",
            "9.255.255.255",
            "11.0.0.0",
            "126.255.255.255",
            "128.0.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "192.167.255.255",
            "192.169.0.0",
            "100.63.255.255",
            "100.128.0.0",
            "169.253.255.255",
            "169.255.0.0",
            "223.255.255.255",
            "240.0.0.1",
            "255.255.255.254",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "fe7f::1",
            "fbff::1",
            "fe00::1",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2a00:1450::1",
        ] {
            assert_eq!(classify(ip(address)), None, "{address}");
        }
    }

    #[test]
    fn an_ipv6_form_of_an_ipv4_address_is_the_kind_of_that_address() {
        use AddressKind::*;
        for (address, kind) in [
            // IPv4-mapped.
            ("::ffff:127.0.0.1", Loopback),
            ("::ffff:10.0.0.1", Private),
            ("::ffff:169.254.169.254", Metadata),
            ("::ffff:100.64.0.1", Shared),
            ("::ffff:0.0.0.0", Unspecified),
            // Translated.
            ("::ffff:0:192.168.0.1", Private),
            // IPv4-compatible, deprecated.
            ("::127.0.0.1", Loopback),
            ("::10.0.0.1", Private),
            ("::169.254.169.254", Metadata),
            // NAT64's well-known prefix.
            ("64:ff9b::127.0.0.1", Loopback),
            ("64:ff9b::a9fe:a9fe", Metadata),
            ("64:ff9b::c0a8:101", Private),
        ] {
            assert_eq!(classify(ip(address)), Some(kind), "{address}");
        }
    }

    #[test]
    fn an_allowance_names_literal_addresses_and_localhost_names_loopback() {
        let none = AddressAllowance::none();
        for address in ["127.0.0.1", "::1", "10.0.0.1", "8.8.8.8"] {
            assert!(!none.allows(ip(address)), "{address}");
        }

        let listed = AddressAllowance::none().with_address(ip("10.0.0.1"));
        assert!(listed.allows(ip("10.0.0.1")));
        assert!(
            listed.allows(ip("::ffff:10.0.0.1")),
            "the mapped form is the address"
        );
        assert!(!listed.allows(ip("10.0.0.2")));
        assert!(
            !listed.allows(ip("127.0.0.1")),
            "a listed address is not loopback"
        );

        let local = AddressAllowance::none().with_localhost();
        for address in ["127.0.0.1", "127.1.2.3", "::1", "::ffff:127.0.0.1"] {
            assert!(local.allows(ip(address)), "{address}");
        }
        for address in ["10.0.0.1", "169.254.169.254", "0.0.0.0", "::", "fe80::1"] {
            assert!(!local.allows(ip(address)), "{address}");
        }
    }

    #[test]
    fn only_a_literal_address_or_localhost_is_an_allowance() {
        let mut allowance = AddressAllowance::none();
        for host in ["LocalHost", "10.0.0.1", "[fd00::1]", "fe80::2"] {
            assert!(allowance.add_host(host), "{host}");
        }
        for host in [
            "*",
            "*.example.com",
            "example.com",
            "localhost.",
            "127.1",
            "[10.0.0.1]x",
        ] {
            assert!(!allowance.add_host(host), "{host}");
        }
        assert!(allowance.allows_localhost());
        assert_eq!(
            allowance.addresses(),
            [ip("10.0.0.1"), ip("fd00::1"), ip("fe80::2")]
        );
    }

    #[test]
    fn each_header_http_sets_and_each_proxy_header_is_refused_in_any_case() {
        // The list is the test's own: a name taken off the module's list
        // must fail here.
        for name in [
            "Host",
            "CONNECTION",
            "keep-alive",
            "Transfer-Encoding",
            "te",
            "TE",
            "Trailer",
            "upgrade",
            "Content-Length",
            "expect",
            "Proxy-Authorization",
            "proxy-connection",
            "PROXY-Anything",
        ] {
            let headers = [(name.to_owned(), "x".to_owned())];
            assert_eq!(
                header_map(&headers).unwrap_err(),
                Refusal::Header {
                    name: name.to_owned()
                },
                "{name}"
            );
        }
        let allowed = [
            ("Authorization", "Bearer t"),
            ("accept", "application/json"),
            ("content-type", "text/plain"),
            ("x-te", "1"),
            ("hostname", "a"),
            ("proxy", "a"),
            ("x-proxy-id", "a"),
            ("x-repeated", "1"),
            ("x-repeated", "2"),
        ]
        .map(|(name, value)| (name.to_owned(), value.to_owned()));
        let map = header_map(&allowed).unwrap();
        assert_eq!(
            map.get_all("x-repeated").iter().count(),
            2,
            "both values are sent"
        );
        assert_eq!(map.len(), allowed.len());
    }

    #[test]
    fn a_header_http_cannot_carry_is_refused_without_its_value() {
        for (name, value) in [("bad name", "x"), ("x-ok", "line\nbreak-secret"), ("", "x")] {
            let refused = header_map(&[(name.to_owned(), value.to_owned())]).unwrap_err();
            assert!(
                matches!(refused, Refusal::InvalidHeader { .. }),
                "{refused:?}"
            );
            assert!(!refused.to_string().contains("secret"), "{refused}");
        }
    }

    #[test]
    fn the_scheme_and_the_host_decide_what_a_url_may_reach() {
        let none = AddressAllowance::none();
        let local = AddressAllowance::none().with_localhost();
        let named = AddressAllowance::none().with_address(ip("10.0.0.5"));

        // `https` to anything; the addresses are checked after the lookup.
        for url in [
            "https://example.com/a?b=c",
            "https://10.0.0.1/",
            "https://localhost/",
            "https://[::1]:8443/",
        ] {
            assert!(Target::parse(url, &none).is_ok(), "{url}");
        }
        // Plain `http` only to an address allowed by name.
        assert!(Target::parse("http://10.0.0.5:8080/", &named).is_ok());
        assert!(Target::parse("http://localhost:8080/", &local).is_ok());
        assert!(Target::parse("http://127.0.0.1:8080/", &local).is_ok());
        for (url, allowance, host) in [
            ("http://example.com/", &none, "example.com"),
            ("http://example.com/", &local, "example.com"),
            ("http://localhost/", &none, "localhost"),
            ("http://localhost/", &named, "localhost"),
            ("http://10.0.0.6/", &named, "10.0.0.6"),
            ("http://8.8.8.8/", &none, "8.8.8.8"),
            ("http://[::1]/", &none, "::1"),
        ] {
            assert_eq!(
                Target::parse(url, allowance).unwrap_err(),
                Refusal::PlainHttp {
                    host: host.to_owned()
                },
                "{url}"
            );
        }
        for (url, scheme) in [
            ("ftp://example.com/", "ftp"),
            ("file:///etc/passwd", "file"),
            ("ws://example.com/", "ws"),
            ("data:text/plain,hi", "data"),
        ] {
            assert_eq!(
                Target::parse(url, &local).unwrap_err(),
                Refusal::Scheme {
                    scheme: scheme.to_owned()
                },
                "{url}"
            );
        }
        for url in [
            "https://user@example.com/",
            "https://user:pass@example.com/",
        ] {
            assert_eq!(Target::parse(url, &none).unwrap_err(), Refusal::Credentials);
        }
        for url in ["not a url", "https://", "https://bücher.example/"] {
            assert!(
                matches!(Target::parse(url, &none), Err(Refusal::Url { .. })),
                "{url}"
            );
        }
    }

    #[test]
    fn every_written_form_of_an_address_is_read_as_the_address() {
        let none = AddressAllowance::none();
        for (url, address) in [
            ("https://127.1/", "127.0.0.1"),
            ("https://0x7f000001/", "127.0.0.1"),
            ("https://2130706433/", "127.0.0.1"),
            ("https://[::ffff:7f00:1]/", "::ffff:127.0.0.1"),
            ("https://[0:0:0:0:0:0:0:1]/", "::1"),
        ] {
            let target = Target::parse(url, &none).unwrap();
            assert!(
                matches!(target.host, Host::Address(found) if found == ip(address)),
                "{url}: {target:?}"
            );
        }
        let named = Target::parse("https://Example.COM./", &none).unwrap();
        assert!(matches!(&named.host, Host::Name(name) if name == "example.com."));
        assert_eq!(named.port, 443);
    }

    fn target(url: &str) -> Target {
        Target::parse(url, &AddressAllowance::none()).unwrap()
    }

    #[test]
    fn one_address_outside_the_internet_refuses_the_whole_name() {
        let host = target("https://example.com:8443/");
        let public = [ip("93.184.216.34"), ip("2606:4700::1")];
        assert_eq!(
            check_addresses(&host, &public, &AddressAllowance::none()).unwrap(),
            [
                SocketAddr::new(public[0], 8443),
                SocketAddr::new(public[1], 8443)
            ]
        );
        for found in [
            [ip("93.184.216.34"), ip("10.0.0.1")],
            [ip("10.0.0.1"), ip("93.184.216.34")],
        ] {
            assert_eq!(
                check_addresses(&host, &found, &AddressAllowance::none()).unwrap_err(),
                Refusal::Address {
                    host: "example.com".to_owned(),
                    address: ip("10.0.0.1"),
                    kind: AddressKind::Private,
                }
            );
        }
        // Named, the same address is reached.
        let named = AddressAllowance::none().with_address(ip("10.0.0.1"));
        assert_eq!(
            check_addresses(&host, &[ip("93.184.216.34"), ip("10.0.0.1")], &named)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn localhost_over_plain_http_reaches_loopback_and_nothing_else() {
        let local = AddressAllowance::none().with_localhost();
        let host = Target::parse("http://localhost:8080/", &local).unwrap();
        assert!(check_addresses(&host, &[ip("127.0.0.1"), ip("::1")], &local).is_ok());
        assert_eq!(
            check_addresses(&host, &[ip("127.0.0.1"), ip("93.184.216.34")], &local).unwrap_err(),
            Refusal::PlainHttp {
                host: "localhost".to_owned()
            }
        );
    }

    #[test]
    fn a_refusal_says_what_and_why_in_words() {
        let refused = Refusal::Address {
            host: "evil.example".to_owned(),
            address: ip("169.254.169.254"),
            kind: AddressKind::Metadata,
        };
        let text = FetchError::from(refused).to_string();
        assert!(text.contains("`evil.example`"), "{text}");
        assert!(text.contains("169.254.169.254"), "{text}");
        assert!(text.contains("metadata"), "{text}");
        assert!(text.contains("allowed by name"), "{text}");
    }

    #[test]
    fn the_pinned_resolver_answers_the_checked_host_and_no_other() {
        use reqwest::dns::Resolve as _;

        let checked = vec![SocketAddr::new(ip("93.184.216.34"), 443)];
        let pinned = Pinned {
            host: "example.com".to_owned(),
            addresses: checked.clone(),
        };
        let name = |text: &str| reqwest::dns::Name::from_str(text).unwrap();
        let found: Vec<SocketAddr> = cineris_testkit::block_on(pinned.resolve(name("example.com")))
            .unwrap()
            .collect();
        assert_eq!(found, checked);
        assert!(cineris_testkit::block_on(pinned.resolve(name("other.example"))).is_err());
        assert!(cineris_testkit::block_on(pinned.resolve(name("localhost"))).is_err());
    }

    #[test]
    fn the_peer_must_be_a_checked_address() {
        let checked = [
            SocketAddr::new(ip("127.0.0.1"), 8080),
            SocketAddr::new(ip("::1"), 8080),
        ];
        let peer = |text: &str| Some(SocketAddr::from_str(text).unwrap());
        assert_eq!(check_peer(peer("127.0.0.1:8080"), &checked), Ok(checked[0]));
        assert!(check_peer(peer("[::ffff:127.0.0.1]:8080"), &checked).is_ok());
        assert!(check_peer(peer("[::1]:8080"), &checked).is_ok());
        for refused in [peer("127.0.0.2:8080"), peer("127.0.0.1:8081"), None] {
            assert_eq!(
                check_peer(refused, &checked),
                Err(Refusal::UncheckedPeer { peer: refused })
            );
        }
    }

    #[test]
    fn the_methods_are_the_six_in_any_case() {
        for method in FetchMethod::ALL {
            assert_eq!(FetchMethod::parse(method.as_str()), Ok(method));
            assert_eq!(
                FetchMethod::parse(&method.as_str().to_ascii_lowercase()),
                Ok(method)
            );
            assert_eq!(method.to_http().as_str(), method.as_str());
        }
        for name in ["CONNECT", "TRACE", "OPTIONS", "", "GETS"] {
            assert!(FetchMethod::parse(name).is_err(), "{name}");
        }
    }

    #[test]
    fn a_request_prints_without_its_header_values() {
        let request = FetchRequest {
            headers: vec![("authorization".to_owned(), "Bearer sk-secret".to_owned())],
            ..FetchRequest::get("https://example.com/?token=secret")
        };
        let printed = format!("{request:?}");
        assert!(!printed.contains("secret"), "{printed}");
        assert!(printed.contains("authorization"), "{printed}");
    }
}
