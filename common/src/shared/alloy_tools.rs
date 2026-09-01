use crate::signer::Signer;
use alloy::{
    network::{Ethereum, EthereumWallet},
    primitives::B256,
    providers::{DynProvider, Provider, ProviderBuilder, WsConnect, ext::DebugApi},
    rpc::types::{Transaction, TransactionRequest, trace::geth::GethDebugTracingOptions},
    signers::local::PrivateKeySigner,
};
use anyhow::Error;
use std::str::FromStr;
use tracing::debug;

pub async fn check_for_revert_reason<P: Provider<Ethereum>>(
    provider: &P,
    tx_hash: B256,
    block_number: u64,
) -> String {
    let default_options = GethDebugTracingOptions::default();
    let trace = provider
        .debug_trace_transaction(tx_hash, default_options)
        .await;

    let trace_errors = if let Ok(trace) = trace {
        find_errors_from_trace(&format!("{trace:?}"))
    } else {
        None
    };

    let tx_details = match provider.get_transaction_by_hash(tx_hash).await {
        Ok(Some(tx)) => tx,
        _ => {
            return format!("Transaction {tx_hash} failed");
        }
    };

    let call_request = get_tx_request_for_call(tx_details);
    let revert_reason = match provider.call(call_request).block(block_number.into()).await {
        Err(e) => e.to_string(),
        Ok(ok) => format!("Unknown revert reason: {ok}"),
    };

    let mut error_msg = format!("Transaction {tx_hash} failed: {revert_reason}");
    if let Some(trace_errors) = trace_errors {
        error_msg.push_str(&trace_errors);
    }
    error_msg
}

fn get_tx_request_for_call(tx_details: Transaction) -> TransactionRequest {
    TransactionRequest::from_transaction(tx_details)
}

fn find_errors_from_trace(trace_str: &str) -> Option<String> {
    let mut start_pos = 0;
    let mut error_message = String::new();
    while let Some(error_start) = trace_str[start_pos..].find("error: Some(") {
        let absolute_pos = start_pos + error_start;
        if let Some(closing_paren) = trace_str[absolute_pos..].find(')') {
            let error_content = &trace_str[absolute_pos..absolute_pos + closing_paren + 1];
            if !error_message.is_empty() {
                error_message.push_str(", ");
            }
            error_message.push_str(error_content);
            start_pos = absolute_pos + closing_paren + 1;
        } else {
            break;
        }
    }
    if !error_message.is_empty() {
        Some(format!(", errors from debug trace: {error_message}"))
    } else {
        None
    }
}

pub async fn construct_alloy_provider(
    signer: &Signer,
    execution_rpc_url: &str,
) -> Result<DynProvider, Error> {
    match signer {
        Signer::PrivateKey(private_key, _) => {
            debug!(
                "Creating alloy provider with URL: {} and private key signer.",
                execution_rpc_url
            );
            let signer = PrivateKeySigner::from_str(private_key.as_str())?;

            Ok(create_alloy_provider_with_wallet(signer.into(), execution_rpc_url).await?)
        }
        Signer::Web3signer(web3signer, address) => {
            debug!(
                "Creating alloy provider with URL: {} and web3signer signer.",
                execution_rpc_url
            );
            let preconfer_address = *address;

            let tx_signer = crate::signer::web3signer::Web3TxSigner::new(
                web3signer.clone(),
                preconfer_address,
            )?;
            let wallet = EthereumWallet::new(tx_signer);

            Ok(create_alloy_provider_with_wallet(wallet, execution_rpc_url).await?)
        }
    }
}

#[derive(Debug)]
pub(crate) enum RpcTransport {
    Http(reqwest::Url),
    WebSocket(reqwest::Url),
}

pub(crate) fn parse_rpc_transport(source: &str, url: &str) -> Result<RpcTransport, Error> {
    if url.trim().is_empty() {
        return Err(anyhow::anyhow!("{source} must not be empty"));
    }

    let parsed = reqwest::Url::parse(url)
        .map_err(|error| anyhow::anyhow!("{source} must be a valid URL: {error}"))?;

    match parsed.scheme() {
        "http" | "https" => Ok(RpcTransport::Http(parsed)),
        "ws" | "wss" => Ok(RpcTransport::WebSocket(parsed)),
        scheme => Err(anyhow::anyhow!(
            "{source} must use the http, https, ws or wss scheme, got '{scheme}'"
        )),
    }
}

async fn create_alloy_provider_with_wallet(
    wallet: EthereumWallet,
    url: &str,
) -> Result<DynProvider, Error> {
    match parse_rpc_transport("RPC URL", url)? {
        RpcTransport::Http(url) => Ok(ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(url)
            .erased()),
        RpcTransport::WebSocket(url) => {
            let ws = WsConnect::new(url.as_str());
            Ok(ProviderBuilder::new()
                .wallet(wallet)
                .connect_ws(ws.clone())
                .await
                .map_err(|e| Error::msg(format!("Execution layer: Failed to connect to WS: {e}")))?
                .erased())
        }
    }
}

pub async fn create_alloy_provider_without_wallet(url: &str) -> Result<DynProvider, Error> {
    match parse_rpc_transport("RPC URL", url)? {
        RpcTransport::Http(url) => Ok(ProviderBuilder::new().connect_http(url).erased()),
        RpcTransport::WebSocket(url) => {
            let ws = WsConnect::new(url.as_str());
            Ok(ProviderBuilder::new()
                .connect_ws(ws.clone())
                .await
                .map_err(|e| Error::msg(format!("Execution layer: Failed to connect to WS: {e}")))?
                .erased())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RpcTransport, create_alloy_provider_without_wallet, parse_rpc_transport};
    use std::time::Duration;
    use tokio::{net::TcpListener, time::timeout};

    #[test]
    fn http_url_with_websocket_text_in_path_uses_http_transport() {
        let transport = parse_rpc_transport("RPC URL", "https://l2.example/ws://archive").unwrap();

        assert!(matches!(transport, RpcTransport::Http(_)));
    }

    #[tokio::test]
    async fn websocket_url_with_whitespace_is_normalized_before_connecting() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let accepted = tokio::spawn(async move {
            matches!(
                timeout(Duration::from_secs(1), listener.accept()).await,
                Ok(Ok(_))
            )
        });

        let url = format!("  ws://{address}  ");
        let _ = timeout(
            Duration::from_secs(1),
            create_alloy_provider_without_wallet(&url),
        )
        .await;

        assert!(
            accepted.await.unwrap(),
            "normalized URL did not reach the WebSocket server"
        );
    }
}
