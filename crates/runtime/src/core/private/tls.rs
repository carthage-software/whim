//! TLS configuration and record-processing built-ins.

use std::cell::OnceCell;
use std::cell::RefCell;
use std::fmt;
use std::io::BufReader;
use std::io::Cursor;
use std::io::ErrorKind;
use std::io::IoSlice;
use std::io::Read;
use std::io::Result as IoResult;
use std::io::Write;
use std::io::empty;
use std::str::from_utf8;
use std::sync::Arc;

use rustls::ClientConfig;
use rustls::ClientConnection;
use rustls::Connection;
use rustls::DigitallySignedStruct;
use rustls::Error as RustlsError;
use rustls::HandshakeKind;
use rustls::ProtocolVersion;
use rustls::RootCertStore;
use rustls::ServerConfig;
use rustls::ServerConnection;
use rustls::SignatureScheme;
use rustls::SupportedProtocolVersion;
use rustls::client::Resumption;
use rustls::client::danger::HandshakeSignatureValid;
use rustls::client::danger::ServerCertVerified;
use rustls::client::danger::ServerCertVerifier;
use rustls::client::verify_server_cert_signed_by_trust_anchor;
use rustls::client::verify_server_name;
use rustls::crypto::CryptoProvider;
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::crypto::verify_tls12_signature;
use rustls::crypto::verify_tls13_signature;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls::pki_types::ServerName;
use rustls::pki_types::UnixTime;
use rustls::server::ClientHello;
use rustls::server::NoServerSessionStorage;
use rustls::server::ParsedCertificate;
use rustls::server::ResolvesServerCert;
use rustls::server::ResolvesServerCertUsingSni;
use rustls::server::WebPkiClientVerifier;
use rustls::sign::CertifiedKey;
use rustls::version::TLS12;
use rustls::version::TLS13;
use x509_parser::certificate::X509Certificate;
use x509_parser::extensions::GeneralName;
use x509_parser::parse_x509_certificate;

use whim_macros::whim_class;
use whim_macros::whim_constant;
use whim_macros::whim_function;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unreachable_invariant;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::dict::DictObject;
use crate::value::dict::keys::KeyRef;
use crate::value::heap::handle::ManagedRef;
use crate::value::string::ByteStringObject;
use crate::value::vec::VecObject;

const TLS_ERROR: &str = "Whim\\_Private\\TlsError";
const TLS_CLIENT_CONFIGURATION: &str = "Whim\\_Private\\TlsClientConfiguration";
const TLS_SERVER_CONFIGURATION: &str = "Whim\\_Private\\TlsServerConfiguration";
const TLS_CONNECTION: &str = "Whim\\_Private\\TlsConnection";
const TLS_VERSIONS_1_2: &[&SupportedProtocolVersion] = &[&TLS12];
const TLS_VERSIONS_1_3: &[&SupportedProtocolVersion] = &[&TLS13];
const TLS_VERSIONS_1_2_AND_1_3: &[&SupportedProtocolVersion] = &[&TLS13, &TLS12];

#[whim_constant("Whim\\_Private\\TLS_VERSION_1_2", "int")]
pub(crate) const TLS_VERSION_1_2: i64 = 12;

#[whim_constant("Whim\\_Private\\TLS_VERSION_1_3", "int")]
pub(crate) const TLS_VERSION_1_3: i64 = 13;

#[whim_constant("Whim\\_Private\\TLS_PEER_VERIFICATION_FULL", "int")]
pub(crate) const TLS_PEER_VERIFICATION_FULL: i64 = 1;

#[whim_constant("Whim\\_Private\\TLS_PEER_VERIFICATION_ALLOW_SELF_SIGNED", "int")]
pub(crate) const TLS_PEER_VERIFICATION_ALLOW_SELF_SIGNED: i64 = 2;

#[whim_constant("Whim\\_Private\\TLS_PEER_VERIFICATION_DISABLED", "int")]
pub(crate) const TLS_PEER_VERIFICATION_DISABLED: i64 = 3;

#[whim_constant("Whim\\_Private\\TLS_CLIENT_AUTHENTICATION_NONE", "int")]
pub(crate) const TLS_CLIENT_AUTHENTICATION_NONE: i64 = 1;

#[whim_constant("Whim\\_Private\\TLS_CLIENT_AUTHENTICATION_OPTIONAL", "int")]
pub(crate) const TLS_CLIENT_AUTHENTICATION_OPTIONAL: i64 = 2;

#[whim_constant("Whim\\_Private\\TLS_CLIENT_AUTHENTICATION_REQUIRED", "int")]
pub(crate) const TLS_CLIENT_AUTHENTICATION_REQUIRED: i64 = 3;

#[whim_constant("Whim\\_Private\\TLS_HANDSHAKE_KIND_FULL", "int")]
pub(crate) const TLS_HANDSHAKE_KIND_FULL: i64 = 1;

#[whim_constant(
    "Whim\\_Private\\TLS_HANDSHAKE_KIND_FULL_WITH_HELLO_RETRY_REQUEST",
    "int"
)]
pub(crate) const TLS_HANDSHAKE_KIND_FULL_WITH_HELLO_RETRY_REQUEST: i64 = 2;

#[whim_constant("Whim\\_Private\\TLS_HANDSHAKE_KIND_RESUMED", "int")]
pub(crate) const TLS_HANDSHAKE_KIND_RESUMED: i64 = 3;

#[whim_constant("Whim\\_Private\\TLS_ERROR_CONFIGURATION", "int")]
pub(crate) const TLS_ERROR_CONFIGURATION: i64 = 1;

#[whim_constant("Whim\\_Private\\TLS_ERROR_CERTIFICATE", "int")]
pub(crate) const TLS_ERROR_CERTIFICATE: i64 = 2;

#[whim_constant("Whim\\_Private\\TLS_ERROR_HANDSHAKE", "int")]
pub(crate) const TLS_ERROR_HANDSHAKE: i64 = 3;

const TLS_ERROR_PROTOCOL: i64 = 4;

#[whim_class("Whim\\_Private\\TlsError", final)]
#[whim_extends("Whim\\Unwind\\Error")]
pub(crate) struct TlsError;

#[whim_methods]
impl TlsError {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("kind(): int")]
    fn kind(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let receiver = cx.receiver();
        cx.get_property(&receiver, "code")
    }
}

#[derive(Debug)]
struct PeerVerifier {
    roots: Arc<RootCertStore>,
    algorithms: WebPkiSupportedAlgorithms,
    policy: i64,
    verify_name: bool,
}

impl ServerCertVerifier for PeerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        if self.policy == TLS_PEER_VERIFICATION_DISABLED {
            return Ok(ServerCertVerified::assertion());
        }

        let certificate = ParsedCertificate::try_from(end_entity)?;
        let verified = verify_server_cert_signed_by_trust_anchor(
            &certificate,
            &self.roots,
            intermediates,
            now,
            self.algorithms.all,
        );
        if verified.is_err()
            && self.policy == TLS_PEER_VERIFICATION_ALLOW_SELF_SIGNED
            && intermediates.is_empty()
        {
            let mut roots = RootCertStore::empty();
            roots.add(end_entity.clone())?;
            verify_server_cert_signed_by_trust_anchor(
                &certificate,
                &roots,
                &[],
                now,
                self.algorithms.all,
            )?;
        } else {
            verified?;
        }

        if self.verify_name {
            verify_server_name(&certificate, server_name)?;
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls12_signature(message, certificate, signature, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls13_signature(message, certificate, signature, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[derive(Debug)]
struct CertificateResolver {
    fallback: Arc<CertifiedKey>,
    named: ResolvesServerCertUsingSni,
}

impl ResolvesServerCert for CertificateResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.named
            .resolve(hello)
            .or_else(|| Some(Arc::clone(&self.fallback)))
    }
}

#[whim_class("Whim\\_Private\\TlsClientConfiguration", final)]
#[derive(Default)]
pub(crate) struct TlsClientConfiguration {
    configuration: OnceCell<Arc<ClientConfig>>,
}

default_built_in_state!(TlsClientConfiguration);

#[whim_methods]
impl TlsClientConfiguration {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "create(bool $useSystemRoots, vec<string> $trustedRoots, null|vec<string> $certificateChain, #[SensitiveParameter] null|string $privateKey, vec<string> $alpnProtocols, null|int $minimumVersion, null|int $maximumVersion, int $peerVerification, bool $verifyPeerName, bool $sendServerName, bool $enableSessionResumption): Whim\\_Private\\TlsClientConfiguration",
        static,
        must_use
    )]
    fn create<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let use_system_roots = arguments.bool(0);
        let trusted_roots = arguments.vec(1);
        let certificate_chain = optional_vec(arguments, 2);
        let private_key = optional_bytes(arguments, 3);
        let alpn_protocols = arguments.vec(4);
        let minimum_version = arguments.optional_int(5);
        let maximum_version = arguments.optional_int(6);
        let peer_verification = arguments.int(7);
        let verify_peer_name = arguments.bool(8);
        let send_server_name = arguments.bool(9);
        let enable_resumption = arguments.bool(10);

        if !matches!(
            peer_verification,
            TLS_PEER_VERIFICATION_FULL
                | TLS_PEER_VERIFICATION_ALLOW_SELF_SIGNED
                | TLS_PEER_VERIFICATION_DISABLED
        ) {
            return Err(tls_error(
                cx,
                TLS_ERROR_CONFIGURATION,
                "unknown peer verification policy",
            ));
        }

        let provider = Arc::new(default_provider());
        let versions = versions(cx, minimum_version, maximum_version)?;
        let roots = roots(cx, use_system_roots, &trusted_roots)?;
        if roots.is_empty() && peer_verification == TLS_PEER_VERIFICATION_FULL {
            return Err(tls_error(
                cx,
                TLS_ERROR_CONFIGURATION,
                "full peer verification requires at least one trusted root",
            ));
        }

        let verifier = Arc::new(PeerVerifier {
            roots: Arc::new(roots),
            algorithms: provider.signature_verification_algorithms,
            policy: peer_verification,
            verify_name: verify_peer_name,
        });
        let builder = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(versions)
            .map_err(|error| configuration_error(cx, error))?
            .dangerous()
            .with_custom_certificate_verifier(verifier);
        let mut configuration = match (certificate_chain, private_key) {
            (None, None) => builder.with_no_client_auth(),
            (Some(chain), Some(key)) => builder
                .with_client_auth_cert(certificates(cx, &chain)?, private_key_der(cx, key)?)
                .map_err(|error| certificate_error(cx, error))?,
            _ => {
                return Err(tls_error(
                    cx,
                    TLS_ERROR_CONFIGURATION,
                    "the client certificate chain and private key must be supplied together",
                ));
            }
        };
        configuration.alpn_protocols = protocols(cx, &alpn_protocols)?;
        configuration.enable_sni = send_server_name;
        if !enable_resumption {
            configuration.resumption = Resumption::disabled();
        }
        build_client_configuration(cx, configuration)
    }

    #[whim_method(
        "createConnection(string $serverName): Whim\\_Private\\TlsConnection",
        must_use
    )]
    fn create_connection<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let name = arguments.bytes(0);
        let name = from_utf8(name).map_err(|_| {
            tls_error(
                cx,
                TLS_ERROR_CONFIGURATION,
                "the TLS server name is not valid UTF-8",
            )
        })?;
        let server_name = ServerName::try_from(name.to_owned())
            .map_err(|error| tls_error(cx, TLS_ERROR_CONFIGURATION, &error.to_string()))?;
        let receiver = cx.receiver();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let state = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&receiver),
                "a TLS client configuration method receives its built-in state",
            )
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let configuration = Arc::clone(unsafe {
            unwrap_option_invariant(
                state.configuration.get(),
                "a TLS client configuration is initialized by its factory",
            )
        });
        let connection = ClientConnection::new(configuration, server_name)
            .map_err(|error| configuration_error(cx, error))?;
        build_connection(
            cx,
            Connection::Client(connection),
            Some(name.as_bytes().to_vec()),
        )
    }
}

#[whim_class("Whim\\_Private\\TlsServerConfiguration", final)]
#[derive(Default)]
pub(crate) struct TlsServerConfiguration {
    configuration: OnceCell<Arc<ServerConfig>>,
}

default_built_in_state!(TlsServerConfiguration);

#[whim_methods]
impl TlsServerConfiguration {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "create(vec<string> $certificateChain, #[SensitiveParameter] string $privateKey, #[SensitiveParameter] dict<string, (vec<string>, string)> $serverNameIdentities, bool $useSystemClientRoots, vec<string> $trustedClientRoots, int $clientAuthentication, vec<string> $alpnProtocols, null|int $minimumVersion, null|int $maximumVersion, bool $enableSessionResumption): Whim\\_Private\\TlsServerConfiguration",
        static,
        must_use
    )]
    fn create<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let certificate_chain = arguments.vec(0);
        let private_key = arguments.bytes(1);
        let identities = arguments.dict(2);
        let use_system_roots = arguments.bool(3);
        let trusted_roots = arguments.vec(4);
        let client_authentication = arguments.int(5);
        let alpn_protocols = arguments.vec(6);
        let minimum_version = arguments.optional_int(7);
        let maximum_version = arguments.optional_int(8);
        let enable_resumption = arguments.bool(9);

        let provider = Arc::new(default_provider());
        let versions = versions(cx, minimum_version, maximum_version)?;
        let builder = ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(versions)
            .map_err(|error| configuration_error(cx, error))?;
        let builder = match client_authentication {
            TLS_CLIENT_AUTHENTICATION_NONE => builder.with_no_client_auth(),
            TLS_CLIENT_AUTHENTICATION_OPTIONAL | TLS_CLIENT_AUTHENTICATION_REQUIRED => {
                let roots = roots(cx, use_system_roots, &trusted_roots)?;
                if roots.is_empty() {
                    return Err(tls_error(
                        cx,
                        TLS_ERROR_CONFIGURATION,
                        "client authentication requires at least one trusted root",
                    ));
                }
                let verifier = WebPkiClientVerifier::builder_with_provider(
                    Arc::new(roots),
                    Arc::clone(&provider),
                );
                let verifier = if client_authentication == TLS_CLIENT_AUTHENTICATION_OPTIONAL {
                    verifier.allow_unauthenticated()
                } else {
                    verifier
                };
                let verifier = verifier
                    .build()
                    .map_err(|error| configuration_error(cx, error))?;
                builder.with_client_cert_verifier(verifier)
            }
            _ => {
                return Err(tls_error(
                    cx,
                    TLS_ERROR_CONFIGURATION,
                    "unknown client authentication policy",
                ));
            }
        };

        let resolver =
            certificate_resolver(cx, &certificate_chain, private_key, &identities, &provider)?;

        let mut configuration = builder.with_cert_resolver(Arc::new(resolver));
        configuration.alpn_protocols = protocols(cx, &alpn_protocols)?;
        if !enable_resumption {
            configuration.session_storage = Arc::new(NoServerSessionStorage {});
            configuration.send_tls13_tickets = 0;
        }
        build_server_configuration(cx, configuration)
    }

    #[whim_method("createConnection(): Whim\\_Private\\TlsConnection", must_use)]
    fn create_connection(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let receiver = cx.receiver();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let state = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&receiver),
                "a TLS server configuration method receives its built-in state",
            )
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let configuration = Arc::clone(unsafe {
            unwrap_option_invariant(
                state.configuration.get(),
                "a TLS server configuration is initialized by its factory",
            )
        });

        let connection =
            ServerConnection::new(configuration).map_err(|error| configuration_error(cx, error))?;
        build_connection(cx, Connection::Server(connection), None)
    }
}

fn certificate_resolver(
    cx: &mut Context<'_, '_, '_>,
    certificate_chain: &ManagedRef<VecObject>,
    private_key: &[u8],
    identities: &ManagedRef<DictObject>,
    provider: &Arc<CryptoProvider>,
) -> Result<CertificateResolver, Throw> {
    let fallback = CertifiedKey::from_der(
        certificates(cx, certificate_chain)?,
        private_key_der(cx, private_key)?,
        provider,
    )
    .map_err(|error| certificate_error(cx, error))?;
    let mut named = ResolvesServerCertUsingSni::new();
    for (name, identity) in identities.iter() {
        let name = key_text(cx, name)?;
        // SAFETY: the surrounding invariant proves this option contains a value.
        let elements = unsafe {
            unwrap_option_invariant(
                identity.as_tuple(),
                "validated TLS server identities contain tuples",
            )
        };
        // SAFETY: the surrounding invariant proves this result is successful.
        let [chain, key] = unsafe {
            unwrap_result_invariant(
                <&[Value; 2]>::try_from(elements.as_slice()),
                "validated TLS server identity tuples contain two elements",
            )
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let chain = unsafe {
            unwrap_option_invariant(
                chain.as_vec(),
                "a validated TLS server identity contains a certificate vec",
            )
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let key = unsafe {
            unwrap_option_invariant(
                key.as_string_bytes(),
                "a validated TLS server identity contains a private-key string",
            )
        };
        let certified = CertifiedKey::from_der(
            certificates(cx, chain)?,
            private_key_der(cx, key)?,
            provider,
        )
        .map_err(|error| certificate_error(cx, error))?;
        named
            .add(&name, certified)
            .map_err(|error| certificate_error(cx, error))?;
    }

    Ok(CertificateResolver {
        fallback: Arc::new(fallback),
        named,
    })
}

struct ConnectionState {
    connection: Connection,
    requested_server_name: Option<Vec<u8>>,
}

impl fmt::Debug for ConnectionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionState")
            .finish_non_exhaustive()
    }
}

#[whim_class("Whim\\_Private\\TlsConnection", final)]
#[derive(Default)]
pub(crate) struct TlsConnection {
    state: RefCell<Option<ConnectionState>>,
}

default_built_in_state!(TlsConnection);

#[whim_methods]
impl TlsConnection {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("isHandshaking(): bool", must_use)]
    fn is_handshaking(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_connection(cx, |_, connection| {
            Ok(Value::bool(connection.connection.is_handshaking()))
        })
    }

    #[whim_method("receiveCiphertext(#[SensitiveParameter] string $bytes): void")]
    fn receive_ciphertext<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let bytes = arguments.bytes(0);
        with_connection(cx, |cx, state| {
            let handshaking = state.connection.is_handshaking();
            let mut input = Cursor::new(bytes);
            // SAFETY: the surrounding invariant proves this result is successful.
            let input_length = unsafe {
                unwrap_result_invariant(
                    u64::try_from(bytes.len()),
                    "a TLS input length fits a cursor position",
                )
            };
            while input.position() < input_length {
                let read = state
                    .connection
                    .read_tls(&mut input)
                    .map_err(|error| tls_error(cx, TLS_ERROR_PROTOCOL, &error.to_string()))?;

                if read == 0 {
                    break;
                }

                state.connection.process_new_packets().map_err(|error| {
                    tls_error(
                        cx,
                        if handshaking {
                            TLS_ERROR_HANDSHAKE
                        } else {
                            TLS_ERROR_PROTOCOL
                        },
                        &error.to_string(),
                    )
                })?;
            }

            Ok(Value::null())
        })
    }

    #[whim_method("receiveEnd(): void")]
    fn receive_end(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_connection(cx, |cx, state| {
            state
                .connection
                .read_tls(&mut empty())
                .map_err(|error| tls_error(cx, TLS_ERROR_PROTOCOL, &error.to_string()))?;
            Ok(Value::null())
        })
    }

    #[whim_method(
        "readPlaintext(Whim\\Refine\\PositiveInt $maximumBytes): null|string",
        must_use
    )]
    fn read_plaintext<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let maximum = arguments.int(0);
        let maximum = positive_size(cx, maximum)?;
        with_connection(cx, |cx, state| {
            let mut bytes = vec![0; maximum];
            match state.connection.reader().read(&mut bytes) {
                Ok(0) => Ok(Value::null()),
                Ok(read) => {
                    bytes.truncate(read);
                    Ok(Value::from_string_vec(cx.vm.heap(), bytes))
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(cx.string(&[])),
                Err(error) => Err(tls_error(cx, TLS_ERROR_PROTOCOL, &error.to_string())),
            }
        })
    }

    #[whim_method("writePlaintext(#[SensitiveParameter] string $bytes): int", must_use)]
    fn write_plaintext<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let bytes = arguments.bytes(0);
        with_connection(cx, |cx, state| {
            let written = state
                .connection
                .writer()
                .write(bytes)
                .map_err(|error| tls_error(cx, TLS_ERROR_PROTOCOL, &error.to_string()))?;

            Ok(Value::int(i64::try_from(written).unwrap_or(i64::MAX)))
        })
    }

    #[whim_method(
        "transmitCiphertext(Whim\\Refine\\PositiveInt $maximumBytes): string",
        must_use
    )]
    fn transmit_ciphertext<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let maximum = arguments.int(0);
        let maximum = positive_size(cx, maximum)?;
        with_connection(cx, |cx, state| {
            let mut output = LimitedWriter::new(maximum);
            state
                .connection
                .write_tls(&mut output)
                .map_err(|error| tls_error(cx, TLS_ERROR_PROTOCOL, &error.to_string()))?;

            Ok(Value::from_string_vec(cx.vm.heap(), output.bytes))
        })
    }

    #[whim_method("sendCloseNotify(): void")]
    fn send_close_notify(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_connection(cx, |_, state| {
            state.connection.send_close_notify();
            Ok(Value::null())
        })
    }

    #[whim_method("protocolVersion(): null|int", must_use)]
    fn protocol_version(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_connection(cx, |_, state| {
            Ok(match state.connection.protocol_version() {
                Some(ProtocolVersion::TLSv1_2) => Value::int(TLS_VERSION_1_2),
                Some(ProtocolVersion::TLSv1_3) => Value::int(TLS_VERSION_1_3),
                Some(_) | None => Value::null(),
            })
        })
    }

    #[whim_method("cipherSuite(): null|string", must_use)]
    fn cipher_suite(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_connection(cx, |cx, state| {
            Ok(state
                .connection
                .negotiated_cipher_suite()
                .map_or_else(Value::null, |suite| {
                    cx.string(format!("{:?}", suite.suite()).as_bytes())
                }))
        })
    }

    #[whim_method("alpnProtocol(): null|string", must_use)]
    fn alpn_protocol(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_connection(cx, |cx, state| {
            Ok(state
                .connection
                .alpn_protocol()
                .map_or_else(Value::null, |protocol| cx.string(protocol)))
        })
    }

    #[whim_method("handshakeKind(): null|int", must_use)]
    fn handshake_kind(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_connection(cx, |_, state| {
            Ok(match state.connection.handshake_kind() {
                Some(HandshakeKind::Full) => Value::int(TLS_HANDSHAKE_KIND_FULL),
                Some(HandshakeKind::FullWithHelloRetryRequest) => {
                    Value::int(TLS_HANDSHAKE_KIND_FULL_WITH_HELLO_RETRY_REQUEST)
                }
                Some(HandshakeKind::Resumed) => Value::int(TLS_HANDSHAKE_KIND_RESUMED),
                None => Value::null(),
            })
        })
    }

    #[whim_method("peerCertificates(): vec<string>", must_use)]
    fn peer_certificates(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_connection(cx, |cx, state| {
            let certificates = state
                .connection
                .peer_certificates()
                .into_iter()
                .flatten()
                .map(|certificate| cx.string(certificate.as_ref()));

            Ok(cx.vec(certificates))
        })
    }

    #[whim_method("serverName(): null|string", must_use)]
    fn server_name(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_connection(cx, |cx, state| {
            let name = match &state.connection {
                Connection::Client(_) => state.requested_server_name.as_deref(),
                Connection::Server(connection) => connection.server_name().map(str::as_bytes),
            };

            Ok(name.map_or_else(Value::null, |name| cx.string(name)))
        })
    }
}

#[whim_function(
    "Whim\\_Private\\tls_decode_certificates(string $bytes): vec<string>",
    must_use
)]
pub(crate) fn decode_certificates<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let certificates = decode_certificates_der(bytes)
        .map_err(|error| tls_error(cx, TLS_ERROR_CERTIFICATE, &error))?
        .into_iter()
        .map(|certificate| cx.string(certificate.as_ref()));

    Ok(cx.vec(certificates))
}

#[whim_function(
    "Whim\\_Private\\tls_certificate_not_before(string $der): int",
    must_use
)]
pub(crate) fn certificate_not_before<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let certificate = parsed_certificate(cx, arguments.bytes(0))?;
    Ok(Value::int(certificate.validity().not_before.timestamp()))
}

#[whim_function(
    "Whim\\_Private\\tls_certificate_not_after(string $der): int",
    must_use
)]
pub(crate) fn certificate_not_after<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let certificate = parsed_certificate(cx, arguments.bytes(0))?;
    Ok(Value::int(certificate.validity().not_after.timestamp()))
}

#[whim_function(
    "Whim\\_Private\\tls_certificate_subject(string $der): string",
    must_use
)]
pub(crate) fn certificate_subject<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let certificate = parsed_certificate(cx, arguments.bytes(0))?;
    Ok(cx.string(certificate.subject().to_string().as_bytes()))
}

#[whim_function(
    "Whim\\_Private\\tls_certificate_issuer(string $der): string",
    must_use
)]
pub(crate) fn certificate_issuer<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let certificate = parsed_certificate(cx, arguments.bytes(0))?;
    Ok(cx.string(certificate.issuer().to_string().as_bytes()))
}

#[whim_function(
    "Whim\\_Private\\tls_certificate_dns_names(string $der): vec<string>",
    must_use
)]
pub(crate) fn certificate_dns_names<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    certificate_alternative_names(cx, arguments.bytes(0), AlternativeNameKind::Dns)
}

#[whim_function(
    "Whim\\_Private\\tls_certificate_ip_addresses(string $der): vec<string>",
    must_use
)]
pub(crate) fn certificate_ip_addresses<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    certificate_alternative_names(cx, arguments.bytes(0), AlternativeNameKind::Ip)
}

#[whim_function(
    "Whim\\_Private\\tls_certificate_email_addresses(string $der): vec<string>",
    must_use
)]
pub(crate) fn certificate_email_addresses<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    certificate_alternative_names(cx, arguments.bytes(0), AlternativeNameKind::Email)
}

#[whim_function(
    "Whim\\_Private\\tls_certificate_uris(string $der): vec<string>",
    must_use
)]
pub(crate) fn certificate_uris<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    certificate_alternative_names(cx, arguments.bytes(0), AlternativeNameKind::Uri)
}

#[whim_function(
    "Whim\\_Private\\tls_decode_private_key(#[SensitiveParameter] string $bytes): string",
    must_use
)]
pub(crate) fn decode_private_key<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let key = decode_private_key_der(bytes)
        .map_err(|error| tls_error(cx, TLS_ERROR_CERTIFICATE, &error))?;

    Ok(cx.string(key.secret_der()))
}

struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl LimitedWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit),
            limit,
        }
    }
}

impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        let count = bytes.len().min(self.limit.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }

    fn write_vectored(&mut self, buffers: &[IoSlice<'_>]) -> IoResult<usize> {
        let mut written = 0;
        for buffer in buffers {
            if self.bytes.len() == self.limit {
                break;
            }
            written += self.write(buffer)?;
        }
        Ok(written)
    }
}

fn with_connection<'call>(
    cx: &mut Context<'call, '_, '_>,
    operation: impl FnOnce(&mut Context<'call, '_, '_>, &mut ConnectionState) -> Result<Value, Throw>,
) -> Result<Value, Throw> {
    let receiver = cx.receiver();
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<TlsConnection>(&receiver),
            "a TLS connection method receives its built-in state",
        )
    };
    let mut state = state.state.borrow_mut();
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state.as_mut(),
            "a TLS connection is initialized by its configuration factory",
        )
    };

    operation(cx, state)
}

fn build_client_configuration(
    cx: &mut Context<'_, '_, '_>,
    configuration: ClientConfig,
) -> Result<Value, Throw> {
    let object = cx.new_built_in_instance(TLS_CLIENT_CONFIGURATION)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<TlsClientConfiguration>(&object),
            "a new TLS client configuration has built-in state",
        )
    };
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            state.configuration.set(Arc::new(configuration)),
            "a new TLS client configuration is not initialized",
        );
    }
    Ok(object)
}

fn build_server_configuration(
    cx: &mut Context<'_, '_, '_>,
    configuration: ServerConfig,
) -> Result<Value, Throw> {
    let object = cx.new_built_in_instance(TLS_SERVER_CONFIGURATION)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<TlsServerConfiguration>(&object),
            "a new TLS server configuration has built-in state",
        )
    };
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            state.configuration.set(Arc::new(configuration)),
            "a new TLS server configuration is not initialized",
        );
    }
    Ok(object)
}

fn build_connection(
    cx: &mut Context<'_, '_, '_>,
    connection: Connection,
    requested_server_name: Option<Vec<u8>>,
) -> Result<Value, Throw> {
    let object = cx.new_built_in_instance(TLS_CONNECTION)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<TlsConnection>(&object),
            "a new TLS connection has built-in state",
        )
    };

    *state.state.borrow_mut() = Some(ConnectionState {
        connection,
        requested_server_name,
    });

    Ok(object)
}

fn versions(
    cx: &mut Context<'_, '_, '_>,
    minimum: Option<i64>,
    maximum: Option<i64>,
) -> Result<&'static [&'static SupportedProtocolVersion], Throw> {
    let minimum = minimum.unwrap_or(TLS_VERSION_1_2);
    let maximum = maximum.unwrap_or(TLS_VERSION_1_3);
    match (minimum, maximum) {
        (TLS_VERSION_1_2, TLS_VERSION_1_2) => Ok(TLS_VERSIONS_1_2),
        (TLS_VERSION_1_3, TLS_VERSION_1_3) => Ok(TLS_VERSIONS_1_3),
        (TLS_VERSION_1_2, TLS_VERSION_1_3) => Ok(TLS_VERSIONS_1_2_AND_1_3),
        _ => Err(tls_error(
            cx,
            TLS_ERROR_CONFIGURATION,
            "the TLS protocol version range is invalid",
        )),
    }
}

fn roots(
    cx: &mut Context<'_, '_, '_>,
    use_system_roots: bool,
    supplied: &ManagedRef<VecObject>,
) -> Result<RootCertStore, Throw> {
    let mut roots = RootCertStore::empty();
    if use_system_roots {
        let loaded = rustls_native_certs::load_native_certs();
        let loaded_any = !loaded.certs.is_empty();
        roots.add_parsable_certificates(loaded.certs);
        if !loaded_any && let Some(error) = loaded.errors.first() {
            return Err(tls_error(cx, TLS_ERROR_CONFIGURATION, &error.to_string()));
        }
    }

    for supplied in supplied.iter() {
        let decoded = decode_certificates_der(string_bytes(supplied))
            .map_err(|error| tls_error(cx, TLS_ERROR_CERTIFICATE, &error))?;
        for certificate in decoded {
            roots
                .add(certificate)
                .map_err(|error| certificate_error(cx, error))?;
        }
    }

    Ok(roots)
}

fn certificates(
    cx: &mut Context<'_, '_, '_>,
    values: &ManagedRef<VecObject>,
) -> Result<Vec<CertificateDer<'static>>, Throw> {
    let mut certificates = Vec::new();
    for value in values.iter() {
        certificates.extend(
            decode_certificates_der(string_bytes(value))
                .map_err(|error| tls_error(cx, TLS_ERROR_CERTIFICATE, &error))?,
        );
    }

    if certificates.is_empty() {
        return Err(tls_error(
            cx,
            TLS_ERROR_CERTIFICATE,
            "the certificate chain is empty",
        ));
    }

    Ok(certificates)
}

fn private_key_der(
    cx: &mut Context<'_, '_, '_>,
    bytes: &[u8],
) -> Result<PrivateKeyDer<'static>, Throw> {
    decode_private_key_der(bytes).map_err(|error| tls_error(cx, TLS_ERROR_CERTIFICATE, &error))
}

fn decode_certificates_der(bytes: &[u8]) -> Result<Vec<CertificateDer<'static>>, String> {
    let certificates = if bytes.starts_with(b"-----BEGIN") {
        let mut reader = BufReader::new(bytes);
        rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    } else if bytes.is_empty() {
        Vec::new()
    } else {
        vec![CertificateDer::from(bytes.to_vec())]
    };

    if certificates.is_empty() {
        return Err("no certificate was found".to_owned());
    }

    for certificate in &certificates {
        ParsedCertificate::try_from(certificate).map_err(|error| error.to_string())?;
    }

    Ok(certificates)
}

#[derive(Clone, Copy)]
enum AlternativeNameKind {
    Dns,
    Ip,
    Email,
    Uri,
}

fn parsed_certificate<'bytes>(
    cx: &mut Context<'_, '_, '_>,
    bytes: &'bytes [u8],
) -> Result<X509Certificate<'bytes>, Throw> {
    let (remaining, certificate) = parse_x509_certificate(bytes)
        .map_err(|error| tls_error(cx, TLS_ERROR_CERTIFICATE, &error.to_string()))?;
    if !remaining.is_empty() {
        return Err(tls_error(
            cx,
            TLS_ERROR_CERTIFICATE,
            "the certificate contains trailing data",
        ));
    }

    Ok(certificate)
}

fn certificate_alternative_names(
    cx: &mut Context<'_, '_, '_>,
    bytes: &[u8],
    kind: AlternativeNameKind,
) -> Result<Value, Throw> {
    let certificate = parsed_certificate(cx, bytes)?;
    let Some(extension) = certificate
        .subject_alternative_name()
        .map_err(|error| certificate_error(cx, error))?
    else {
        return Ok(cx.vec([]));
    };

    let mut values = Vec::new();
    for name in &extension.value.general_names {
        let bytes = match (kind, name) {
            (AlternativeNameKind::Dns, GeneralName::DNSName(name))
            | (AlternativeNameKind::Email, GeneralName::RFC822Name(name))
            | (AlternativeNameKind::Uri, GeneralName::URI(name)) => name.as_bytes(),
            (AlternativeNameKind::Ip, GeneralName::IPAddress(address)) => {
                if !matches!(address.len(), 4 | 16) {
                    return Err(tls_error(
                        cx,
                        TLS_ERROR_CERTIFICATE,
                        "the certificate contains an invalid IP subject alternative name",
                    ));
                }
                address
            }
            (_, GeneralName::Invalid(_, _)) => {
                return Err(tls_error(
                    cx,
                    TLS_ERROR_CERTIFICATE,
                    "the certificate contains an invalid subject alternative name",
                ));
            }
            _ => continue,
        };
        if bytes.is_empty() {
            return Err(tls_error(
                cx,
                TLS_ERROR_CERTIFICATE,
                "the certificate contains an empty subject alternative name",
            ));
        }
        values.push(cx.string(bytes));
    }

    Ok(cx.vec(values))
}

fn decode_private_key_der(bytes: &[u8]) -> Result<PrivateKeyDer<'static>, String> {
    if bytes.starts_with(b"-----BEGIN") {
        let mut reader = BufReader::new(bytes);
        return rustls_pemfile::private_key(&mut reader)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "no private key was found".to_owned());
    }

    PrivateKeyDer::try_from(bytes.to_vec()).map_err(str::to_owned)
}

fn protocols(
    cx: &mut Context<'_, '_, '_>,
    values: &ManagedRef<VecObject>,
) -> Result<Vec<Vec<u8>>, Throw> {
    let protocols = values
        .iter()
        .map(|value| string_bytes(value).to_vec())
        .collect::<Vec<_>>();
    if protocols
        .iter()
        .any(|protocol| protocol.is_empty() || protocol.len() > usize::from(u8::MAX))
    {
        return Err(tls_error(
            cx,
            TLS_ERROR_CONFIGURATION,
            "an ALPN protocol must contain between 1 and 255 bytes",
        ));
    }

    Ok(protocols)
}

fn string_bytes(value: &Value) -> &[u8] {
    // SAFETY: the surrounding invariant proves this option contains a value.
    unsafe {
        unwrap_option_invariant(
            value.as_string_bytes(),
            "a validated string vec contains only strings",
        )
    }
}

fn optional_vec(arguments: Arguments<'_>, index: usize) -> Option<ManagedRef<VecObject>> {
    match arguments.get(index) {
        None => None,
        Some(value) if value.is_null() => None,
        Some(_) => Some(arguments.vec(index)),
    }
}

fn optional_bytes(arguments: Arguments<'_>, index: usize) -> Option<&[u8]> {
    match arguments.get(index) {
        None => None,
        Some(value) if value.is_null() => None,
        Some(_) => Some(arguments.bytes(index)),
    }
}

fn key_text(cx: &mut Context<'_, '_, '_>, key: KeyRef<'_>) -> Result<String, Throw> {
    let text = match key {
        KeyRef::String(string) => {
            from_utf8(ByteStringObject::handle_bytes(string)).map(str::to_owned)
        }
        KeyRef::ShortString(string) => from_utf8(string.as_bytes()).map(str::to_owned),
        // SAFETY: the surrounding invariant makes this path unreachable.
        KeyRef::Int(_) | KeyRef::Bool(_) => unsafe {
            unreachable_invariant("validated TLS server identity keys are strings")
        },
    };

    text.map_err(|_| cx.type_error("a server name identity is not valid UTF-8"))
}

fn positive_size(cx: &mut Context<'_, '_, '_>, value: i64) -> Result<usize, Throw> {
    usize::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| cx.type_error("a maximum byte count must be positive"))
}

fn tls_error(cx: &mut Context<'_, '_, '_>, kind: i64, message: &str) -> Throw {
    let class = cx.vm.intern(TLS_ERROR.as_bytes());
    cx.vm.throw(class, message, kind)
}

fn configuration_error(cx: &mut Context<'_, '_, '_>, error: impl fmt::Display) -> Throw {
    tls_error(cx, TLS_ERROR_CONFIGURATION, &error.to_string())
}

fn certificate_error(cx: &mut Context<'_, '_, '_>, error: impl fmt::Display) -> Throw {
    tls_error(cx, TLS_ERROR_CERTIFICATE, &error.to_string())
}
