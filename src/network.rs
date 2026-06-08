extern crate reqwest;

use std::thread;
use std::time::Duration;
use log::{error, warn, info, debug};
use std::array;
use std::error::Error;
use std::io::Bytes;

use nom::AsBytes;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT, CONTENT_TYPE, CONNECTION, InvalidHeaderValue};

pub struct NetworkParameters{
    pub user_agent: String,
    pub retry_times: usize,
    pub wait_time_min: usize,
    pub wait_time_max: usize,
    pub fetch_timeout: usize,
    pub connect_timeout: usize,
    pub proxy_server: Option<String>,
    pub referrer_url: Option<String>,
    /// Global toggle for honoring robots.txt; can be turned off via config `respect_robots_txt = false`.
    /// Per-site `SiteConfig::respect_robots` settings are still ANDed with this.
    pub respect_robots_txt: bool,
    /// Minimum number of seconds to wait between consecutive fetches to the same host.
    /// Overrides the default (`wait_time_min`) when set via config `min_host_interval_sec`.
    pub min_host_interval_sec: Option<usize>,
}

pub fn read_network_parameters(app_config: &config::Config) -> NetworkParameters {
    let mut net_params = NetworkParameters{
        user_agent: String::new(),
        retry_times: 3,
        wait_time_min: 2,
        wait_time_max: 5,
        fetch_timeout: 60,
        connect_timeout: 60,
        proxy_server: None,
        referrer_url: None,
        respect_robots_txt: true,
        min_host_interval_sec: None,
    };

    match app_config.get_int("fetch_timeout") {
        Ok(config_timeout) =>{
            if config_timeout > 0{
                net_params.fetch_timeout = config_timeout.unsigned_abs() as usize;
            }
        },
        Err(ex) =>{
            info!("Using default timeout of {} due to error fetching timeout from config: {}",
                net_params.fetch_timeout,
                ex)
        }
    }

    match app_config.get_string("user_agent") {
        Ok(user_agent_configured) => {
            net_params.user_agent.clear();
            net_params.user_agent.push_str(&user_agent_configured);
        },
        Err(e) => {
            error!("When extracting user agent from config: {:?}", e)
        }
    }

    match app_config.get_string("proxy_server_url") {
        Ok(proxy_server_url) => {
            net_params.proxy_server = Some(proxy_server_url);
        },
        Err(e) => {
            info!("Could not identify proxy server url from config, not using proxy, message: {:?}", e)
        }
    }

    match app_config.get_bool("respect_robots_txt") {
        Ok(respect_flag) => {
            net_params.respect_robots_txt = respect_flag;
        },
        Err(_) => {
            info!("Using default of respecting robots.txt (set respect_robots_txt = false in config to disable)");
        }
    }

    match app_config.get_int("min_host_interval_sec") {
        Ok(interval_secs) => {
            if interval_secs >= 0 {
                net_params.min_host_interval_sec = Some(interval_secs as usize);
            }
        },
        Err(_) => {
            info!("Using default per-host crawl interval (set min_host_interval_sec in config to override)");
        }
    }

    return net_params;
}

pub fn make_http_client(netw_params: &NetworkParameters) -> reqwest::blocking::Client {

    let pool_idle_timeout: u64 = 90;
    let pool_max_idle_connections: usize = 1;

    // add headers
    let mut headers = HeaderMap::new();
    if let Some(referrer) = netw_params.referrer_url.clone() {
        match HeaderValue::from_str(referrer.as_str()) {
            Ok(header_referrer) => headers.insert(reqwest::header::REFERER, header_referrer),
            Err(_e) => headers.insert(reqwest::header::REFERER, HeaderValue::from_static("https://www.google.com/"))
        };
    }

    // add do not track:
    headers.insert(reqwest::header::DNT, HeaderValue::from(1));
    headers.insert(reqwest::header::CONNECTION, HeaderValue::from_static("keep-alive"));
    headers.insert(reqwest::header::ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.5"));
    // Only advertise encodings that reqwest can decompress (gzip feature enabled; brotli/zstd are not)
    headers.insert(reqwest::header::ACCEPT_ENCODING, HeaderValue::from_static("gzip, deflate"));
    headers.insert(reqwest::header::ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml,application/json"));

    let client_bld: reqwest::blocking::ClientBuilder= reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(netw_params.fetch_timeout as u64))
        .user_agent(netw_params.user_agent.clone())
        .default_headers(headers)
        .gzip(true)
        // Persist cookies across requests within this client. Sites such as NSE/BSE set
        // session cookies on the landing page that are required for subsequent API/file
        // requests; without a cookie store those requests are served an HTML error page.
        .cookie_store(true)
        .pool_idle_timeout(Duration::from_secs(pool_idle_timeout))
        .pool_max_idle_per_host(pool_max_idle_connections);

    if netw_params.proxy_server.is_some() {
        if let Some(proxy_url_str) = netw_params.proxy_server.clone() {
            // if proxy is configured, then add proxy with https rule:
            match reqwest::Proxy::https(proxy_url_str.as_str()) {
                Ok(proxy_obj) => {
                    let client: reqwest::blocking::Client = client_bld
                        .proxy(proxy_obj)
                        .build()
                        .expect("Require valid parameters for building HTTP client");
                    return client;
                }
                Err(e) => {
                    info!("Unable to use proxy; when setting the proxy server, message was: {}", e);
                }
            }
        }
    }

    let client_no_proxy: reqwest::blocking::Client = client_bld
        .build()
        .expect("Require valid parameters for building HTTP client");

    return client_no_proxy;
}


pub fn build_llm_api_client(connect_timeout: u64, fetch_timeout: u64, proxy_url: Option<String>, custom_headers: Option<HeaderMap>) -> reqwest::blocking::Client {

    let pool_idle_timeout: u64 = (connect_timeout + fetch_timeout) * 5;
    let pool_max_idle_connections: usize = 1;

    let mut headers = HeaderMap::new();
    if let Some(custom_header_map) = custom_headers {
        headers = custom_header_map;
    }
    // prepare headers:
    headers.insert(reqwest::header::CONNECTION, HeaderValue::from_static("keep-alive"));
    headers.insert(reqwest::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));

    // build client:
    let client_builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(fetch_timeout))
        .connect_timeout(Duration::from_secs(connect_timeout))
        .default_headers(headers)
        .gzip(true)
        .pool_idle_timeout(Duration::from_secs(pool_idle_timeout))
        .pool_max_idle_per_host(pool_max_idle_connections);
    if proxy_url.is_some() {
        if let Some(proxy_url_str) = proxy_url {
            // if proxy is configured, then add proxy with https rule:
            match reqwest::Proxy::https(proxy_url_str.as_str()) {
                Ok(proxy_obj) => {
                    let client: reqwest::blocking::Client = client_builder
                        .proxy(proxy_obj)
                        .build()
                        .expect("Require valid parameters for building HTTP client");
                    return client;
                }
                Err(e) => {
                    error!("Unable to use proxy, Error when setting the proxy server: {}", e);
                }
            }
        }
    }
    let client_no_proxy: reqwest::blocking::Client = client_builder
        .build()
        .expect("Require valid parameters for building REST API client");
    return client_no_proxy;
}


pub fn http_get_binary(website_url: &String, client: &reqwest::blocking::Client) -> bytes::Bytes {
    let retry_times = 3;
    let wait_time = 2;

    for attempt_no in 0..retry_times {
        log::info!("HTTP GET waiting for {} sec", wait_time);
        thread::sleep(Duration::from_secs(wait_time));

        let req_builder = client.get(website_url);
        match req_builder.send() {
            Ok(resp) => {
                match resp.bytes() {
                    Ok(binary_data) => {
                        log::debug!("HTTP GET retrieved bytes array of length: {}", binary_data.len());
                        return binary_data;
                    },
                    Err(ex) => {
                        log::error!("Failed attempt #{}, When retrieving binary data from HTTP GET: {:?}", attempt_no, ex.to_string());
                        log::info!("HTTP GET waiting for an additional {} sec", wait_time);
                        thread::sleep(Duration::from_secs(wait_time));
                    }
                }
            }
            Err(e) => {
                log::error!("Failed attempt #{}, When executing binary HTTP GET on url {}, error: {:?}", attempt_no, website_url, e.to_string());
                log::info!("HTTP GET waiting for an additional {} sec", wait_time);
                thread::sleep(Duration::from_secs(wait_time));
            }
        }
    }
    return bytes::Bytes::new();
}

/// Returns true for HTTP status codes that are worth retrying.
/// Permanent client errors (4xx) are not retried, with the exception of
/// 408 (Request Timeout) and 429 (Too Many Requests). Server errors (5xx) are retried.
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    if status.is_server_error() {
        return true;
    }
    matches!(status.as_u16(), 408 | 429)
}

/// Cheap dependency-free jitter in the range [0, 1000) milliseconds, derived from the
/// system clock's sub-second component. Spreads out retries to avoid a thundering herd.
fn backoff_jitter_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.subsec_nanos() as u64) % 1000)
        .unwrap_or(0)
}

pub fn http_get(website_url: &String, client: &reqwest::blocking::Client, retry_times: usize, wait_time: usize) -> String {

    let max_attempts = retry_times.max(1);

    for attempt_no in 0..max_attempts {

        // Politeness delay before the first request; exponential backoff (capped) before retries.
        let sleep_secs = if attempt_no == 0 {
            wait_time as u64
        } else {
            let backoff = (wait_time as u64).saturating_mul(1u64 << attempt_no.min(5));
            backoff.min(60)
        };
        if sleep_secs > 0 {
            log::debug!("HTTP GET waiting for {} sec before attempt {}", sleep_secs, attempt_no + 1);
            thread::sleep(Duration::from_secs(sleep_secs));
            if attempt_no > 0 {
                thread::sleep(Duration::from_millis(backoff_jitter_ms()));
            }
        }

        let req_builder = client.get(website_url);
        match req_builder.send() {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    match resp.text() {
                        Ok(http_response_body_text) => {
                            log::debug!("From HTTP response, got text of length: {}", http_response_body_text.len());
                            return http_response_body_text;
                        }
                        Err(ex) => {
                            log::error!("HTTP GET attempt {}: status {} but failed to read body of {}: {:?}",
                                attempt_no + 1, status, website_url, ex.to_string());
                        }
                    }
                } else if is_retryable_status(status) {
                    log::warn!("HTTP GET attempt {}: retryable status {} for {}",
                        attempt_no + 1, status, website_url);
                } else {
                    // Permanent error (e.g. 403, 404, 410): do not retry.
                    log::warn!("HTTP GET: non-retryable status {} for {} — giving up.", status, website_url);
                    return String::from("");
                }
            }
            Err(e) => {
                log::error!("HTTP GET failed attempt no {}: When retrieving url {}: {:?}", attempt_no + 1, website_url, e.to_string());
            }
        }
    }
    // Return blank string, if nothing works
    String::from("")
}


#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn test_server_errors_are_retryable() {
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_status(StatusCode::GATEWAY_TIMEOUT));
    }

    #[test]
    fn test_rate_limit_and_timeout_are_retryable() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));      // 429
        assert!(is_retryable_status(StatusCode::REQUEST_TIMEOUT));        // 408
    }

    #[test]
    fn test_permanent_client_errors_not_retryable() {
        assert!(!is_retryable_status(StatusCode::FORBIDDEN));             // 403
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));             // 404
        assert!(!is_retryable_status(StatusCode::GONE));                  // 410
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));          // 401
    }

    #[test]
    fn test_success_is_not_retryable() {
        // success codes never go through the retry path
        assert!(!is_retryable_status(StatusCode::OK));
        assert!(!is_retryable_status(StatusCode::NO_CONTENT));
    }

    #[test]
    fn test_backoff_jitter_within_bounds() {
        for _ in 0..100 {
            assert!(backoff_jitter_ms() < 1000);
        }
    }
}
