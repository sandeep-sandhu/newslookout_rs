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


pub fn make_http_client(fetch_timeout: u64, user_agent: &str, base_url: String, proxy_url: Option<String>) -> reqwest::blocking::Client {

    let pool_idle_timeout: u64 = 90;
    let pool_max_idle_connections: usize = 1;

    // add headers
    let mut headers = HeaderMap::new();
    match HeaderValue::from_str(base_url.as_str()) {
        Ok(header_referrer) => headers.insert(reqwest::header::REFERER, header_referrer),
        Err(e) => headers.insert(reqwest::header::REFERER, HeaderValue::from_static("https://www.google.com/"))
    };
    // add do not track:
    headers.insert(reqwest::header::DNT, HeaderValue::from(1));
    headers.insert(reqwest::header::CONNECTION, HeaderValue::from_static("keep-alive"));

    let client_bld: reqwest::blocking::ClientBuilder= reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(fetch_timeout))
        .user_agent(user_agent.to_string())
        .default_headers(headers)
        .gzip(true)
        .pool_idle_timeout(Duration::from_secs(pool_idle_timeout))
        .pool_max_idle_per_host(pool_max_idle_connections);

    if proxy_url.is_some() {
        if let Some(proxy_url_str) = proxy_url {
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
                    error!("Unable to use proxy, Error when setting the proxy server: {}", e);
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

pub fn http_get(website_url: &String, client: &reqwest::blocking::Client, retry_times: u64, wait_time: u64) -> String {

    for attempt_no in 0..retry_times {

        log::info!("HTTP GET waiting for {} sec", wait_time);
        thread::sleep(Duration::from_secs(wait_time));

        let req_builder = client.get(website_url);
        match req_builder.send() {
            Ok(resp) => {
                match resp.text() {
                    Ok(http_response_body_text) => {
                        log::debug!("From HTTP response, got text of length: {}", http_response_body_text.len());
                        return http_response_body_text;
                    }
                    Err(ex) => {
                        log::error!("HTTP GET failed attempt no {}: When retrieving {}, error: {:?}", attempt_no+1, website_url, ex.to_string());
                    }
                }
            }
            Err(e) => {
                log::error!("HTTP GET failed attempt no {}: When retrieving url {}: {:?}", attempt_no, website_url, e.to_string());
            }
        }
    }
    // Return blank string, if nothing works
    String::from("")
}


#[cfg(test)]
mod tests {
    use crate::network;

    #[test]
    fn test_1() {
        // TODO: implement this
        assert_eq!(1, 1);
    }
}
