use core::str::from_utf8;
#[cfg(feature = "debug")]
use defmt::{error, info};
#[cfg(not(feature = "debug"))]
use crate::log_noop::{error, info};
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_net::Stack;


use embassy_rp::clocks::RoscRng;
use embassy_time::Duration;

use rand::RngCore;
use reqwless::client::{HttpClient, TlsConfig, TlsVerify};
use reqwless::request::{Method, RequestBuilder};
#[cfg(feature = "debug")]
use {defmt_rtt as _, panic_probe as _};





pub async fn make_api_request<'a>(
    stack: Stack<'static>,
    rx_buffer: &'a mut [u8; 2400],
    url: &'a str,
) -> &'a str {
    let client_state = TcpClientState::<1, 1024, 1024>::new();
    let mut tcp_client = TcpClient::new(stack, &client_state);
    tcp_client.set_timeout(Some(Duration::from_secs(5)));
    let dns_client = DnsSocket::new(stack);
    let mut tls_read_buffer = [0; 8000];
    let mut tls_write_buffer = [0; 8000];
    let mut rng = RoscRng;
    // Generate random seed
    let seed = rng.next_u64();

    let tls_config = TlsConfig::new(seed, &mut tls_read_buffer, &mut tls_write_buffer, TlsVerify::None);

    let mut http_client = HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);
    info!("connecting to {}", &url);
    let Ok(req_handle) = http_client.request(Method::GET, url).await else {
        error!("Failed to make request!");
        return "";
    };

    let mut request = req_handle
        .content_type(reqwless::headers::ContentType::TextPlain);
    let response_fut = request.send(rx_buffer);

    let Ok(response) = response_fut.await else {
        error!("Failed to get response result!");
        return "";
    };

    let body = match from_utf8(response.body().read_to_end().await.unwrap()) {
        Ok(b) => b,
        Err(_e) => {
            error!("Failed to read response body");
            return "";
        }
    };
    info!("Response body: {:?}", body);
    body
}
