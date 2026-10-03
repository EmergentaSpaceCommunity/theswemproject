//! The Workbench served at an address, for whoever comes from elsewhere.
//!
//! Behind the door are a terminal and a person's keys, so the door refuses
//! to be careless. An address a browser opens from elsewhere is a name
//! reached over TLS: the Workbench holds a certificate of its own, or the
//! person's proxy stands in front of it on this machine. Whatever else is
//! asked for is refused here, in words, whoever asks - the command or a
//! product the harness is built into.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::pem::PemObject as _;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use url::Url;

use super::Assembled;
use crate::workbench_shell::{Listening, Way, WorkbenchShellHandle};
use crate::{Access, WorkbenchShellError};

/// How the way to the Workbench is kept closed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Closed {
    /// By a certificate of its own, read from these files and read again
    /// when they change.
    Certificate { chain: PathBuf, key: PathBuf },
    /// By the person's proxy, which stands in front on this machine.
    ProxyInFront,
    /// It is `localhost`: sign-in tried where the Workbench runs.
    ThisMachine,
}

/// Where and how to serve.
#[derive(Clone, Debug)]
pub struct ServeAt {
    /// What a browser opens.
    pub address: Url,
    /// Where the Workbench listens.
    pub listen: SocketAddr,
    pub closed: Closed,
    /// Where a browser finds what is drawn for Apps. An origin of its own:
    /// an App is somebody else's page.
    pub apps_address: Url,
    pub apps_listen: SocketAddr,
    pub apps_bundle: Option<PathBuf>,
}

/// A Workbench served at an address.
pub struct ServedAt {
    pub handle: WorkbenchShellHandle,
    /// What a browser opens.
    pub address: String,
    /// The word of a first start, when this Workbench belongs to nobody
    /// yet. Said once, to whoever started it.
    pub word: Option<String>,
    /// Until when the certificate is good, when the Workbench holds one.
    pub certificate_good_until: Option<SystemTime>,
}

fn origin_of(address: &Url) -> String {
    address.origin().ascii_serialization()
}

fn only_an_origin(what: &str, address: &Url) -> Result<(), String> {
    if address.host_str().is_none()
        || !matches!(address.path(), "" | "/")
        || address.query().is_some()
        || address.fragment().is_some()
        || !address.username().is_empty()
    {
        return Err(format!(
            "{what} is given as a name and nothing after it, as https://workbench.example.org; \
             {address} is more than that."
        ));
    }
    Ok(())
}

impl ServeAt {
    /// Whether this can be served without being careless, and why not.
    ///
    /// # Errors
    ///
    /// What is refused, in words a person reads where they started it.
    pub fn looked_at(&self) -> Result<Way, String> {
        only_an_origin("The address", &self.address)?;
        only_an_origin("Where Apps are drawn", &self.apps_address)?;
        if origin_of(&self.address) == origin_of(&self.apps_address) {
            return Err(format!(
                "Apps are drawn at an address of their own: an App is somebody else's page and \
                 must not be the Workbench's. Both were given as {}.",
                origin_of(&self.address)
            ));
        }
        let way = match (&self.closed, self.address.scheme()) {
            (Closed::ThisMachine, "http") => {
                if self.address.host_str() != Some("localhost") {
                    return Err(format!(
                        "An address a browser opens from elsewhere begins with https://. \
                         {} does not, and it is not localhost.",
                        self.address
                    ));
                }
                Way::ThisMachine
            }
            (Closed::ThisMachine, _) => {
                return Err(format!(
                    "{} is reached over TLS: say where its certificate is, or that a proxy \
                     stands in front.",
                    self.address
                ));
            }
            (Closed::Certificate { .. }, "https") => Way::Certificate,
            (Closed::ProxyInFront, "https") => Way::ProxyInFront,
            (_, scheme) => {
                return Err(format!(
                    "An address a browser opens from elsewhere begins with https://, not \
                     {scheme}://. The way to a Workbench is not left open."
                ));
            }
        };
        if way != Way::Certificate {
            for (what, listen) in [("the Workbench", self.listen), ("Apps", self.apps_listen)] {
                if !listen.ip().is_loopback() {
                    return Err(format!(
                        "Without a certificate of its own {what} listens on this machine alone, \
                         not on {listen}: what would be said there could be read on the way. \
                         A proxy in front is what listens elsewhere."
                    ));
                }
            }
        }
        if self.apps_address.scheme() != self.address.scheme() {
            return Err(format!(
                "Where Apps are drawn is reached the way the Workbench is: {} is not.",
                self.apps_address
            ));
        }
        Ok(way)
    }
}

/// A certificate read from files, and read again when they change: a
/// certificate is renewed by something else, and the Workbench is not
/// restarted for it.
struct FromFiles {
    chain: PathBuf,
    key: PathBuf,
    held: RwLock<Held>,
}

struct Held {
    certified: Arc<rustls::sign::CertifiedKey>,
    changed: Option<SystemTime>,
    looked: Instant,
}

/// The files are looked at no more often than this.
const LOOKED_EVERY: Duration = Duration::from_mins(1);

impl std::fmt::Debug for FromFiles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FromFiles")
            .field("chain", &self.chain)
            .finish_non_exhaustive()
    }
}

fn changed(chain: &Path, key: &Path) -> Option<SystemTime> {
    let of = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|found| found.modified())
            .ok()
    };
    of(chain).max(of(key))
}

fn certified(chain: &Path, key: &Path) -> Result<rustls::sign::CertifiedKey, String> {
    let certificates = CertificateDer::pem_file_iter(chain)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|error| {
            format!(
                "the certificate at {} cannot be read: {error}",
                chain.display()
            )
        })?;
    if certificates.is_empty() {
        return Err(format!("{} holds no certificate", chain.display()));
    }
    let key_read = PrivateKeyDer::from_pem_file(key)
        .map_err(|error| format!("the key at {} cannot be read: {error}", key.display()))?;
    let signing = rustls::crypto::ring::sign::any_supported_type(&key_read)
        .map_err(|error| format!("the key at {} is of no kind known: {error}", key.display()))?;
    let certified = rustls::sign::CertifiedKey::new(certificates, signing);
    certified
        .keys_match()
        .map_err(|error| format!("the certificate and the key do not belong together: {error}"))?;
    Ok(certified)
}

impl FromFiles {
    fn read(chain: &Path, key: &Path) -> Result<Self, String> {
        Ok(Self {
            chain: chain.to_owned(),
            key: key.to_owned(),
            held: RwLock::new(Held {
                certified: Arc::new(certified(chain, key)?),
                changed: changed(chain, key),
                looked: Instant::now(),
            }),
        })
    }
}

impl rustls::server::ResolvesServerCert for FromFiles {
    fn resolve(
        &self,
        _hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        {
            let held = self.held.read().ok()?;
            if held.looked.elapsed() < LOOKED_EVERY {
                return Some(Arc::clone(&held.certified));
            }
        }
        let mut held = self.held.write().ok()?;
        held.looked = Instant::now();
        let now = changed(&self.chain, &self.key);
        if now != held.changed {
            // A certificate half written is not taken: what was held
            // stays until what is there can be read whole.
            if let Ok(renewed) = certified(&self.chain, &self.key) {
                held.certified = Arc::new(renewed);
                held.changed = now;
            }
        }
        Some(Arc::clone(&held.certified))
    }
}

fn tls_of(closed: &Closed) -> Result<Option<Arc<rustls::ServerConfig>>, String> {
    let Closed::Certificate { chain, key } = closed else {
        return Ok(None);
    };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut served = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(FromFiles::read(chain, key)?));
    served.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Some(Arc::new(served)))
}

impl Assembled {
    /// Open the Workbench door at an address: whoever comes signs in.
    ///
    /// # Errors
    ///
    /// What was asked would leave the way open, the certificate cannot be
    /// read, the book of who may come in cannot be opened, or a listener
    /// cannot be bound.
    pub async fn serve_at(self, at: ServeAt) -> Result<ServedAt, String> {
        let way = at.looked_at()?;
        let tls = tls_of(&at.closed)?;
        let certificate = match &at.closed {
            Closed::Certificate { chain, .. } => Some(chain.clone()),
            _ => None,
        };
        let certificate_good_until = certificate
            .as_deref()
            .and_then(crate::workbench_shell::certificate_good_until);
        self.name_the_owner().await?;
        let failed = |error: WorkbenchShellError| error.to_string();
        let owner = self.state.owner_name().await.map_err(failed)?;
        let access = Arc::new(
            Access::open(&self.root.access(), &at.address, &owner)
                .map_err(|error| error.to_string())?,
        );
        let word = access
            .word_of_a_first_start()
            .map_err(|error| error.to_string())?;
        // A secret of the run is minted all the same: the state asks for
        // one wherever it is not served at an address, and it never leaves
        // this process.
        self.state.set_session_token(crate::mint_session_token()?);
        self.state
            .serve_at(access, way, Some(origin_of(&at.apps_address)), certificate);
        // The channels a person added start with the product too.
        self.state
            .enable_channels(&self.root.channels())
            .map_err(|error| error.to_string())?;
        self.state
            .enable_tunnels(&self.root.tunnels())
            .map_err(|error| error.to_string())?;
        self.state
            .keep_time(&self.root.schedules())
            .await
            .map_err(failed)?;
        self.state.look_at_the_machine_meanwhile();
        self.state.take_up_chats().await.map_err(failed)?;
        let handle = crate::serve_workbench(
            Arc::clone(&self.state),
            Listening {
                bind: at.listen,
                sandbox_bind: at.apps_listen,
                sandbox_at: Some(origin_of(&at.apps_address)),
                tls,
                apps_bundle: at.apps_bundle,
            },
        )
        .await?;
        Ok(ServedAt {
            handle,
            address: origin_of(&at.address),
            word,
            certificate_good_until,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use url::Url;

    use super::{Closed, ServeAt};
    use crate::workbench_shell::Way;

    fn asked(address: &str, apps: &str, listen: &str, closed: Closed) -> ServeAt {
        let listen: SocketAddr = listen.parse().expect("where to listen");
        ServeAt {
            address: Url::parse(address).expect("an address"),
            listen,
            closed,
            apps_address: Url::parse(apps).expect("an address"),
            apps_listen: SocketAddr::new(listen.ip(), listen.port() + 1),
            apps_bundle: None,
        }
    }

    fn certificate() -> Closed {
        Closed::Certificate {
            chain: "chain.pem".into(),
            key: "key.pem".into(),
        }
    }

    #[test]
    fn a_name_over_tls_is_served_with_a_certificate_or_behind_a_proxy() {
        assert_eq!(
            asked(
                "https://workbench.example.org",
                "https://apps.workbench.example.org",
                "0.0.0.0:443",
                certificate()
            )
            .looked_at(),
            Ok(Way::Certificate)
        );
        assert_eq!(
            asked(
                "https://workbench.example.org",
                "https://apps.workbench.example.org",
                "127.0.0.1:8787",
                Closed::ProxyInFront
            )
            .looked_at(),
            Ok(Way::ProxyInFront)
        );
        assert_eq!(
            asked(
                "http://localhost:8787",
                "http://localhost:8788",
                "127.0.0.1:8787",
                Closed::ThisMachine
            )
            .looked_at(),
            Ok(Way::ThisMachine)
        );
    }

    #[test]
    fn the_way_is_never_left_open() {
        for (address, apps, listen, closed, why) in [
            // No TLS, from elsewhere.
            (
                "http://workbench.example.org",
                "http://apps.workbench.example.org",
                "0.0.0.0:80",
                Closed::ThisMachine,
                "https://",
            ),
            (
                "http://workbench.example.org",
                "http://apps.workbench.example.org",
                "127.0.0.1:8787",
                Closed::ProxyInFront,
                "https://",
            ),
            // A name over TLS and nothing that speaks TLS.
            (
                "https://workbench.example.org",
                "https://apps.workbench.example.org",
                "0.0.0.0:443",
                Closed::ThisMachine,
                "certificate",
            ),
            // A proxy in front, and the Workbench listening where the
            // proxy should.
            (
                "https://workbench.example.org",
                "https://apps.workbench.example.org",
                "0.0.0.0:8787",
                Closed::ProxyInFront,
                "this machine alone",
            ),
            // localhost, listened for from elsewhere.
            (
                "http://localhost:8787",
                "http://localhost:8788",
                "0.0.0.0:8787",
                Closed::ThisMachine,
                "this machine alone",
            ),
            // Apps where the Workbench is.
            (
                "https://workbench.example.org",
                "https://workbench.example.org",
                "0.0.0.0:443",
                certificate(),
                "of their own",
            ),
            // More than a name.
            (
                "https://workbench.example.org/swem",
                "https://apps.workbench.example.org",
                "0.0.0.0:443",
                certificate(),
                "nothing after it",
            ),
        ] {
            let refused = asked(address, apps, listen, closed)
                .looked_at()
                .expect_err(address);
            assert!(refused.contains(why), "{address}: {refused}");
        }
    }
}
