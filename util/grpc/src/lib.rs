// Copyright (c) 2018-2021 The MobileCoin Foundation

//! Utilities related to grpc bindings, particularly, setting up routes,
//! creating grpc error objects, and various common services like the admin
//! service and the health check service

#![deny(missing_docs)]

mod autogenerated_code {
    pub use protobuf::well_known_types::Empty;

    /// Needed due to how to the auto-generated code references the Empty
    /// message.
    pub mod empty {
        pub use protobuf::well_known_types::Empty;
    }

    // Include the auto-generated code.
    include!(concat!(env!("OUT_DIR"), "/protos-auto-gen/mod.rs"));
}
pub use autogenerated_code::*;

mod admin_server;
mod admin_service;
mod auth;
mod build_info_service;
mod cookie_helper;
mod grpcio_extensions;
mod health_service;
mod retry_config;
mod server_cert_reloader;

pub use crate::{
    admin_server::AdminServer,
    admin_service::{AdminService, GetConfigJsonFn},
    auth::{
        AnonymousAuthenticator, Authenticator, AuthenticatorError, AuthorizationHeaderError,
        BasicCredentials, TokenAuthenticator, TokenBasicCredentialsGenerator,
        TokenBasicCredentialsGeneratorError, ANONYMOUS_USER,
    },
    autogenerated_code::*,
    build_info_service::BuildInfoService,
    cookie_helper::{Error as CookieError, GrpcCookieStore},
    grpcio_extensions::{ConnectionUriGrpcioChannel, ConnectionUriGrpcioServer},
    health_service::{HealthCheckStatus, HealthService, ReadinessIndicator},
    retry_config::GrpcRetryConfig,
    server_cert_reloader::{ServerCertReloader, ServerCertReloaderError},
};

use futures::prelude::*;
use grpcio::{RpcContext, RpcStatus, RpcStatusCode, UnarySink};
use mc_common::logger::{log, o, Level, Logger};
use mc_util_metrics::SVC_COUNTERS;
use rand::Rng;
use std::{
    fmt::Display,
    sync::atomic::{AtomicU64, Ordering},
};

/// Helper which reduces boilerplate when implementing grpc API traits.
#[inline]
pub fn send_result<T>(
    ctx: RpcContext,
    sink: UnarySink<T>,
    resp: Result<T, RpcStatus>,
    logger: &Logger,
) {
    let logger = logger.clone();
    let success = resp.is_ok();
    let mut code = RpcStatusCode::OK;

    match resp {
        Ok(ok) => ctx.spawn(
            sink.success(ok)
                .map_err(move |err| log::error!(logger, "failed to reply: {}", err))
                .map(|_| ()),
        ),
        Err(e) => {
            code = e.code();
            ctx.spawn(
                sink.fail(e)
                    .map_err(move |err| log::error!(logger, "failed to reply: {}", err))
                    .map(|_| ()),
            )
        }
    }

    SVC_COUNTERS.resp(&ctx, success);
    SVC_COUNTERS.status_code(&ctx, code);
}

macro_rules! report_err_with_code(
    ($context:expr, $err:expr, $code:expr, $logger:expr, $log_level:expr) => {{
        let err_str = format!("{}: {}", $context, $err);
        log::log!($logger, $log_level, "", "{}", err_str);
        RpcStatus::with_message($code, err_str)
    }}
);

/// Database errors are mapped to "Internal Error" and logged at error level
#[inline]
pub fn rpc_database_err<E: Display>(err: E, logger: &Logger) -> RpcStatus {
    report_err_with_code!(
        "Database Error",
        err,
        RpcStatusCode::INTERNAL,
        logger,
        Level::Error
    )
}

/// More general helpers which reduces boilerplate when reporting errors.
/// The type of the error doesn't always indicate what kind of error code to
/// use. For instance deserialization might sometimes be
/// invalid input and sometimes an internal or database error.
#[inline]
pub fn rpc_internal_error<S: Display, E: Display>(
    context: S,
    err: E,
    logger: &Logger,
) -> RpcStatus {
    report_err_with_code!(context, err, RpcStatusCode::INTERNAL, logger, Level::Error)
}

/// Invalid arg is listed at debug level, because it can be triggered by bad
/// clients, and may not indicate an actionable issue with the servers.
#[inline]
pub fn rpc_invalid_arg_error<S: Display, E: Display>(
    context: S,
    err: E,
    logger: &Logger,
) -> RpcStatus {
    report_err_with_code!(
        context,
        err,
        RpcStatusCode::INVALID_ARGUMENT,
        logger,
        Level::Debug
    )
}

/// Permissions error is listed at debug level, because it can be triggered by
/// clients in normal operation, and may not indicate an actionable issue with
/// the servers.
#[inline]
pub fn rpc_permissions_error<S: Display, E: Display>(
    context: S,
    err: E,
    logger: &Logger,
) -> RpcStatus {
    report_err_with_code!(
        context,
        err,
        RpcStatusCode::PERMISSION_DENIED,
        logger,
        Level::Debug
    )
}

/// Out-of-range error occurs when a client makes a request that is out of
/// bounds. This is a separate error from invalid arg because it may help them
/// handle the error more easily.
/// This is logged at debug level because it likely doesn't indicate an
/// actionable issue with the servers.
#[inline]
pub fn rpc_out_of_range_error<S: Display, E: Display>(
    context: S,
    err: E,
    logger: &Logger,
) -> RpcStatus {
    report_err_with_code!(
        context,
        err,
        RpcStatusCode::OUT_OF_RANGE,
        logger,
        Level::Debug
    )
}

/// Precondition error occurs when a client makes a request that can't be
/// satisfied for the server's current state. For example, trying to activate an
/// ingest server that is already activated might return this error.
///
/// This is logged at info level so that it is visible to the operator, but
/// doesn't trigger an alert or indicate a problem that requires action to
/// address.
#[inline]
pub fn rpc_precondition_error<S: Display, E: Display>(
    context: S,
    err: E,
    logger: &Logger,
) -> RpcStatus {
    report_err_with_code!(
        context,
        err,
        RpcStatusCode::FAILED_PRECONDITION,
        logger,
        Level::Info
    )
}

/// Unavailable error may be returned if e.g. an rpc call fails but could
/// succeed if it is retried.
///
/// GRPC-core offers the following guidance:
///
/// Service implementors can use the following guidelines to decide between
/// FAILED_PRECONDITION, ABORTED, and UNAVAILABLE:
/// (a) Use UNAVAILABLE if the client can retry just the failing call.
/// (b) Use ABORTED if the client should retry at a higher level (e.g., when a
/// client-specified test-and-set fails, indicating the client should restart a
/// read-modify-write sequence).
/// (c) Use FAILED_PRECONDITION if the client should not retry until the system
/// state has been explicitly fixed. E.g., if an "rmdir" fails because the
/// directory is non-empty, FAILED_PRECONDITION should be returned since the
/// client should not retry unless the files are deleted from the directory.
#[inline]
pub fn rpc_unavailable_error<S: Display, E: Display>(
    context: S,
    err: E,
    logger: &Logger,
) -> RpcStatus {
    report_err_with_code!(
        context,
        err,
        RpcStatusCode::UNAVAILABLE,
        logger,
        Level::Debug
    )
}

/// Converts a serialization Error to an RpcStatus error.
pub fn ser_to_rpc_err(error: mc_util_serial::encode::Error, logger: &Logger) -> RpcStatus {
    rpc_internal_error("Serialization", error, logger)
}

/// Converts a deserialization Error to an RpcStatus error.
pub fn deser_to_rpc_err(error: mc_util_serial::decode::Error, logger: &Logger) -> RpcStatus {
    rpc_internal_error("Deserialization", error, logger)
}

/// Converts an encode Error to an RpcStatus error.
pub fn encode_to_rpc_err(error: mc_util_serial::EncodeError, logger: &Logger) -> RpcStatus {
    rpc_internal_error("Encode", error, logger)
}

/// Converts a decode Error to an RpcStatus error.
pub fn decode_to_rpc_err(error: mc_util_serial::DecodeError, logger: &Logger) -> RpcStatus {
    rpc_internal_error("Decode", error, logger)
}

/// Helper for running a server around an instance of grpc API implementation
/// Can be reused for many endpoints
/// Handles a bunch of grpc boilerplate that was being copy pasted
use grpcio::{Server, Service};

/// Build and start a server composed of several services
#[inline]
pub fn run_server(
    env: std::sync::Arc<grpcio::Environment>,
    services: Vec<Service>,
    port: u16,
    logger: &Logger,
) -> Server {
    use grpcio::ServerBuilder;

    // FIXME: This should default to localhost and you should have to provide the IP
    let mut server = ServerBuilder::new(env);

    for service in services {
        server = server.register_service(service);
    }

    let mut server = server.bind("0.0.0.0", port).build().unwrap();
    server.start();
    for (host, port) in server.bind_addrs() {
        log::info!(logger, "API listening on {}:{}", host, port);
    }
    server
}

/// A utility method for injecting peer information into a logger, ideally
/// making it easier to debug RPC-related interactions.
pub fn rpc_logger(ctx: &RpcContext, logger: &Logger) -> Logger {
    let hash =
        mc_common::fast_hash(format!("{}{}", *RPC_LOGGER_CLIENT_ID_SEED, ctx.peer()).as_bytes());
    let hash_str = hex_fmt::HexFmt(hash).to_string();

    let request_id = RPC_LOGGER_REQUEST_ID_COUNTER.fetch_add(1, Ordering::SeqCst);

    logger.new(o!("rpc_client_id" => hash_str, "rpc_request_id" => request_id))
}

lazy_static::lazy_static! {
    // Generate a random seed at startup so that rpc_client_id hashes are not identifying specific
    // users by leaking IP addresses.
    static ref RPC_LOGGER_CLIENT_ID_SEED: String = {
        let mut rng = rand::thread_rng();
        std::iter::repeat(())
            .map(|()| rng.sample(rand::distributions::Alphanumeric))
            .take(32)
            .map(char::from)
            .collect()
    };

    static ref RPC_LOGGER_REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);
}
