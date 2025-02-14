#![cfg(feature = "quic")]
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
};

use anyhow::Context;
use quic_rpc::{transport::QuinnChannelTypes, RpcClient, RpcServer};
use quinn::{ClientConfig, Endpoint, ServerConfig};
use tokio::task::JoinHandle;

mod math;
use math::*;
mod util;
use util::*;

/// Constructs a QUIC endpoint configured for use a client only.
///
/// ## Args
///
/// - server_certs: list of trusted certificates.
#[allow(unused)]
pub fn make_client_endpoint(
    bind_addr: SocketAddr,
    server_certs: &[&[u8]],
) -> anyhow::Result<Endpoint> {
    let client_cfg = configure_client(server_certs)?;
    let mut endpoint = Endpoint::client(bind_addr)?;
    endpoint.set_default_client_config(client_cfg);
    Ok(endpoint)
}

/// Constructs a QUIC endpoint configured to listen for incoming connections on a certain address
/// and port.
///
/// ## Returns
///
/// - a stream of incoming QUIC connections
/// - server certificate serialized into DER format
#[allow(unused)]
pub fn make_server_endpoint(bind_addr: SocketAddr) -> anyhow::Result<(Endpoint, Vec<u8>)> {
    let (server_config, server_cert) = configure_server()?;
    let endpoint = Endpoint::server(server_config, bind_addr)?;
    Ok((endpoint, server_cert))
}

/// Builds default quinn client config and trusts given certificates.
///
/// ## Args
///
/// - server_certs: a list of trusted certificates in DER format.
fn configure_client(server_certs: &[&[u8]]) -> anyhow::Result<ClientConfig> {
    let mut certs = rustls::RootCertStore::empty();
    for cert in server_certs {
        certs.add(&rustls::Certificate(cert.to_vec()))?;
    }

    Ok(ClientConfig::with_root_certificates(certs))
}

/// Returns default server configuration along with its certificate.
#[allow(clippy::field_reassign_with_default)] // https://github.com/rust-lang/rust-clippy/issues/6527
fn configure_server() -> anyhow::Result<(ServerConfig, Vec<u8>)> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
    let cert_der = cert.serialize_der()?;
    let priv_key = cert.serialize_private_key_der();
    let priv_key = rustls::PrivateKey(priv_key);
    let cert_chain = vec![rustls::Certificate(cert_der.clone())];

    let mut server_config = ServerConfig::with_single_cert(cert_chain, priv_key)?;
    Arc::get_mut(&mut server_config.transport)
        .unwrap()
        .max_concurrent_uni_streams(0_u8.into());

    Ok((server_config, cert_der))
}

pub struct Endpoints {
    client: Endpoint,
    server: Endpoint,
    server_addr: SocketAddr,
}

pub fn make_endpoints() -> anyhow::Result<Endpoints> {
    let server_addr: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 12345));
    let (server, server_certs) = make_server_endpoint(server_addr)?;
    let client = make_client_endpoint("0.0.0.0:0".parse()?, &[&server_certs])?;
    Ok(Endpoints {
        client,
        server,
        server_addr,
    })
}

fn run_server(server: quinn::Endpoint) -> JoinHandle<anyhow::Result<()>> {
    tokio::task::spawn(async move {
        let connection = quic_rpc::transport::quinn::ServerChannel::new(
            server.accept().await.context("accept failed")?.await?,
        );
        let server = RpcServer::<ComputeService, QuinnChannelTypes>::new(connection);
        ComputeService::server(server).await?;
        anyhow::Ok(())
    })
}

#[tokio::test]
async fn quinn_channel_bench() -> anyhow::Result<()> {
    type C = QuinnChannelTypes;
    let Endpoints {
        client,
        server,
        server_addr,
    } = make_endpoints()?;
    let server_handle = run_server(server);
    let client = client.connect(server_addr, "localhost")?.await?;
    let client = quic_rpc::transport::quinn::ClientChannel::new(client);
    let client = RpcClient::<ComputeService, C>::new(client);
    bench(client, 50000).await?;
    println!("waiting for server");
    check_termination_anyhow::<C>(server_handle).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn quinn_channel_smoke() -> anyhow::Result<()> {
    type C = QuinnChannelTypes;
    let Endpoints {
        client,
        server,
        server_addr,
    } = make_endpoints()?;
    let server_handle = run_server(server);
    let client_connection = client.connect(server_addr, "localhost")?.await?;
    let client_connection = quic_rpc::transport::quinn::ClientChannel::new(client_connection);
    smoke_test::<C>(client_connection).await?;
    check_termination_anyhow::<C>(server_handle).await?;
    Ok(())
}
