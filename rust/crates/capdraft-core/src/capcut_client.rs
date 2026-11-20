//! CapCut `common_task` TTS requests: query, headers, SSML, signatures
//! (port of the parts of `external/capcut-tts-api/capcut_common_task_client.py`
//! the app uses: `tts-new` and `tts-query`; STT and upload stay out, ADR-017).

use crate::ids::{new_uuid_lower, now_unix, random_bytes, uuid_hex};
use crate::json::{Object, Value, dumps_compact};
use crate::jv;
use crate::pyval::{PyResult, str_of};
use base64::Engine as _;
use md5::{Digest, Md5};
use num_bigint::BigUint;
use std::path::Path;
use std::sync::LazyLock;

pub const BASE: &str = "https://editor-api-sg.capcutapi.com";

pub const TTS_SIGN_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAmTd34Lw4b7IuldSXh/zY
CMla+ITdGG5TeWz6ad+OySd4r+IrY45AoqrYUxhQ2dl+7z+i7r/5vEa8rr39BYfB
8AGMQLmZA8HmgpWBsqrn/V6daUALkKnkLb70Fn32CJigIuGXAYqxUdGuI340aC+0
v5Es3puJsHyzf01/AelE4Cdc6bZhQrASJLBh8R3BQToYClmDVSDUQk28o8sl/guA
Z4n303Vj+6Siv1HayPCdV6kpVVnMBAG4+umUbwGmn132N3fgpzLarFF3XyWmS1zh
D/J07iM/rP8GDO9IskHNHd2phrO0G6KzrcFAnTBHjVv+hCBEfzN/no3FNA9AuC36
mwIDAQAB
-----END PUBLIC KEY-----";

const USER_AGENT: &str = "Cronet/TTNetVersion:1d7cc3b1 2025-07-16 QuicVersion:52c2b40d 2025-04-03";

/// Which device the app presents to CapCut (ADR-018).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceProfile {
    /// The Python app's client: `MacBookPro17,1`; per chunk `device_id` = `tdid`
    /// and a separate `iid`, 19 digits each; polls with the fixed default device.
    Legacy,
    /// Upstream `K07VN/capcut-tts-api` (2026-10): `MacBookPro17,4`; per chunk one
    /// 20-digit id for `device_id`, `iid` and `tdid`; polls with that same device.
    Upstream,
}

impl DeviceProfile {
    /// `tts_device_profile` in `config.json`: `legacy`, else upstream.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        if s.trim().eq_ignore_ascii_case("legacy") {
            Self::Legacy
        } else {
            Self::Upstream
        }
    }

    #[must_use]
    pub const fn model(self) -> &'static str {
        match self {
            Self::Legacy => "MacBookPro17,1",
            Self::Upstream => "MacBookPro17,4",
        }
    }
}

/// `DEFAULT_DEVICE` of `profile` (ids as in the Python client; the TTS
/// wrapper replaces them per chunk).
#[must_use]
pub fn default_device_for(profile: DeviceProfile) -> Object {
    let mut d = default_device();
    d.insert("device_type", Value::from(profile.model()));
    d.insert("device_brand", Value::from(profile.model()));
    d
}

/// `DEFAULT_DEVICE`, in the original's key order.
#[must_use]
pub fn default_device() -> Object {
    let mut o = Object::new();
    for (k, v) in [
        ("aid", "359289"),
        ("app_name", "CapCut"),
        ("appvr", "8.7.0"),
        ("version_name", "8.7.0"),
        ("version_code", "8.7.0"),
        ("channel", "capcutpc_google"),
        ("device_platform", "mac"),
        ("device_type", "MacBookPro17,1"),
        ("device_brand", "MacBookPro17,1"),
        ("os_version", "15.7.4"),
        ("device_id", "7647183892936328721"),
        ("iid", "7647185302080423697"),
        ("region", "VN"),
        ("loc", "VN"),
        ("lan", "vi-VN"),
        ("pf", "3"),
        ("tdid", "7647183892936328721"),
    ] {
        o.insert(k, Value::from(v));
    }
    o
}

/// `DEFAULT_DEVICE` updated with `overrides` (later keys win, new keys appended).
#[must_use]
pub fn device_with(overrides: Option<&Object>) -> Object {
    device_for(DeviceProfile::Legacy, overrides)
}

/// [`default_device_for`] updated with `overrides`.
#[must_use]
pub fn device_for(profile: DeviceProfile, overrides: Option<&Object>) -> Object {
    let mut d = default_device_for(profile);
    if let Some(o) = overrides {
        for (k, v) in o {
            d.insert(k.clone(), v.clone());
        }
    }
    d
}

/// `DEFAULT_DEVICE` updated with a device JSON file.
///
/// # Errors
/// File and JSON errors; `AttributeError` when the file is not an object.
pub fn load_device(path: Option<&Path>) -> PyResult<Object> {
    let Some(p) = path else {
        return Ok(default_device());
    };
    match crate::draft_reader::read_json_file(p)? {
        Value::Object(o) => Ok(device_with(Some(&o))),
        other => Err(crate::pyval::no_attr(&other, "keys")),
    }
}

fn dev<'a>(device: &'a Object, k: &str) -> PyResult<&'a Value> {
    device.get(k).ok_or_else(|| format!("KeyError: '{k}'"))
}

fn dev_str(device: &Object, k: &str) -> PyResult<String> {
    dev(device, k).map(str_of)
}

#[must_use]
pub fn md5_hex(data: &[u8]) -> String {
    Md5::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `escape_xml`: `& < > " '` as entities.
#[must_use]
pub fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// `urllib.parse.quote_plus`.
#[must_use]
pub fn quote_plus(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3 / 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `urllib.parse.urlencode` of ordered pairs.
#[must_use]
pub fn urlencode(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Query parameters every `common_task` call carries.
///
/// # Errors
/// `KeyError` when the device lacks a field.
pub fn common_query(
    device: &Object,
    babi_param: Option<&Value>,
    include_region: bool,
) -> PyResult<Vec<(String, String)>> {
    let mut q = Vec::new();
    for k in [
        "app_name",
        "device_type",
        "os_version",
        "channel",
        "version_name",
        "device_brand",
        "device_id",
        "iid",
        "version_code",
        "device_platform",
        "aid",
    ] {
        q.push((k.to_owned(), dev_str(device, k)?));
    }
    if include_region {
        q.push(("region".into(), dev_str(device, "region")?));
    }
    if let Some(b) = babi_param {
        q.push(("babi_param".into(), dumps_compact(b)));
    }
    Ok(q)
}

/// `00-<32 hex>-<16 hex>-01`.
#[must_use]
pub fn make_trace_id() -> String {
    let seed = uuid_hex();
    format!("00-{seed}-{}-01", &seed[..16])
}

/// Headers of a `common_task` call (values as the device holds them).
///
/// # Errors
/// `KeyError` when the device lacks a field; `AttributeError` when `loc` is not a string.
pub fn base_headers(
    device: &Object,
    body_text: &str,
    appid: bool,
) -> PyResult<Vec<(String, Value)>> {
    let now = Value::from((now_unix() as i64).to_string());
    let loc = dev(device, "loc")?
        .as_str()
        .ok_or_else(|| crate::pyval::no_attr(&Value::Null, "lower"))?
        .to_lowercase();
    let mut h: Vec<(String, Value)> = vec![
        ("content-type".into(), Value::from("application/json")),
        ("appvr".into(), dev(device, "appvr")?.clone()),
        ("ch".into(), dev(device, "channel")?.clone()),
        ("device-time".into(), now.clone()),
        ("lan".into(), dev(device, "lan")?.clone()),
        ("loc".into(), dev(device, "loc")?.clone()),
        ("pf".into(), dev(device, "pf")?.clone()),
        ("sign-ver".into(), Value::from("1")),
        ("tdid".into(), dev(device, "tdid")?.clone()),
        (
            "x-ss-stub".into(),
            Value::from(md5_hex(body_text.as_bytes())),
        ),
        ("x-ss-dp".into(), dev(device, "aid")?.clone()),
        ("x-khronos".into(), now),
        ("x-tt-trace-id".into(), Value::from(make_trace_id())),
        ("user-agent".into(), Value::from(USER_AGENT)),
        ("accept-encoding".into(), Value::from("gzip, deflate")),
        ("store-country-code".into(), Value::from(loc)),
        ("store-country-code-src".into(), Value::from("did")),
        ("is-dispatch-us-ttp".into(), Value::from("0")),
        ("is-app-region-us-ttp".into(), Value::from("0")),
    ];
    if appid {
        h.push(("app-sdk-version".into(), dev(device, "appvr")?.clone()));
        h.push(("appid".into(), dev(device, "aid")?.clone()));
    }
    Ok(h)
}

/// md5 of `9e2c|<last 7 chars of the path>|3|appvr|device_time|tdid|11ac`.
#[must_use]
pub fn make_sign_header(url: &str, appvr: &str, device_time: &str, tdid: &str) -> String {
    let path = url.split('?').next().unwrap_or("");
    let chars: Vec<char> = path.chars().collect();
    let tail: String = chars[chars.len().saturating_sub(7)..].iter().collect();
    md5_hex(format!("9e2c|{tail}|3|{appvr}|{device_time}|{tdid}|11ac").as_bytes())
}

// ------------------------------------------------------------------ RSA

fn der_len(data: &[u8], pos: usize) -> PyResult<(usize, usize)> {
    let first = *data.get(pos).ok_or("IndexError: index out of range")?;
    if first < 0x80 {
        return Ok((usize::from(first), pos + 1));
    }
    let n = usize::from(first & 0x7f);
    let bytes = data
        .get(pos + 1..pos + 1 + n)
        .ok_or("IndexError: index out of range")?;
    Ok((
        bytes
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b)),
        pos + 1 + n,
    ))
}

fn der_value(data: &[u8], pos: usize, tag: u8) -> PyResult<(&[u8], usize)> {
    let got = *data.get(pos).ok_or("IndexError: index out of range")?;
    if got != tag {
        return Err(format!(
            "ValueError: bad DER tag: expected 0x{tag:02x}, got 0x{got:02x}"
        ));
    }
    let (len, p) = der_len(data, pos + 1)?;
    let v = data.get(p..p + len).ok_or("ValueError: truncated DER")?;
    Ok((v, p + len))
}

/// `(modulus, exponent)` of a PEM `SubjectPublicKeyInfo` RSA key.
///
/// # Errors
/// `ValueError` for malformed keys.
pub fn rsa_public_numbers_from_pem(pem: &str) -> PyResult<(BigUint, BigUint)> {
    let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("binascii.Error: {e}"))?;
    let (outer, pos) = der_value(&der, 0, 0x30)?;
    if pos != der.len() {
        return Err("ValueError: trailing data in public key".into());
    }
    let (_, pos) = der_value(outer, 0, 0x30)?;
    let (bits, pos) = der_value(outer, pos, 0x03)?;
    if pos != outer.len() || bits.first() != Some(&0) {
        return Err("ValueError: bad subjectPublicKeyInfo".into());
    }
    let (seq, pos) = der_value(&bits[1..], 0, 0x30)?;
    if pos != bits.len() - 1 {
        return Err("ValueError: trailing data in RSA public key".into());
    }
    let (m, pos) = der_value(seq, 0, 0x02)?;
    let (e, pos) = der_value(seq, pos, 0x02)?;
    if pos != seq.len() {
        return Err("ValueError: trailing integer data in RSA public key".into());
    }
    Ok((BigUint::from_bytes_be(m), BigUint::from_bytes_be(e)))
}

static TTS_KEY: LazyLock<PyResult<(BigUint, BigUint)>> =
    LazyLock::new(|| rsa_public_numbers_from_pem(TTS_SIGN_PUBLIC_KEY_PEM));

/// RSA PKCS#1 v1.5 encryption with the TTS signing key, base64.
///
/// # Errors
/// `ValueError` when the message does not fit the key.
pub fn rsa_encrypt_pkcs1v15(message: &[u8]) -> PyResult<String> {
    let (n, e) = TTS_KEY.as_ref().map_err(Clone::clone)?;
    let key_len = (n.bits() as usize).div_ceil(8);
    if message.len() > key_len.saturating_sub(11) {
        return Err("ValueError: message too long for RSA PKCS#1 v1.5".into());
    }
    let ps_len = key_len - message.len() - 3;
    let mut ps: Vec<u8> = Vec::with_capacity(ps_len);
    while ps.len() < ps_len {
        let chunk = random_bytes(ps_len - ps.len());
        ps.extend(chunk.into_iter().filter(|b| *b != 0));
    }
    let mut encoded = Vec::with_capacity(key_len);
    encoded.extend_from_slice(&[0, 2]);
    encoded.extend_from_slice(&ps[..ps_len]);
    encoded.push(0);
    encoded.extend_from_slice(message);
    let c = BigUint::from_bytes_be(&encoded).modpow(e, n).to_bytes_be();
    let mut out = vec![0u8; key_len - c.len()];
    out.extend_from_slice(&c);
    Ok(base64::engine::general_purpose::STANDARD.encode(out))
}

/// The `sign` field of a TTS payload.
///
/// # Errors
/// As [`rsa_encrypt_pkcs1v15`].
pub fn make_tts_payload_sign(
    ssml: &str,
    extra_info: Option<&str>,
    device_id: &str,
    app_id: &str,
) -> PyResult<String> {
    let mut input = format!(
        "appid:{app_id}&did:{device_id}&creditDisable:false&ssml:{}",
        md5_hex(ssml.as_bytes())
    );
    if let Some(x) = extra_info {
        input.push_str(&format!("&extraInfo:{x}"));
    }
    rsa_encrypt_pkcs1v15(input.as_bytes())
}

// ------------------------------------------------------------------ bodies

/// `(babi_param, body)` of a `tts-new` call: one `<voice>` block per text.
///
/// # Errors
/// `KeyError` for a missing device field; RSA errors.
pub fn tts_new_body(
    texts: &[String],
    voice: &str,
    resource_id: &str,
    rate: &str,
    device: &Object,
) -> PyResult<(Value, Value)> {
    tts_new_body_for(
        DeviceProfile::Legacy,
        texts,
        voice,
        resource_id,
        rate,
        device,
    )
}

/// [`tts_new_body`]; the upstream profile also XML-escapes the voice and resource id.
///
/// # Errors
/// As [`tts_new_body`].
pub fn tts_new_body_for(
    profile: DeviceProfile,
    texts: &[String],
    voice: &str,
    resource_id: &str,
    rate: &str,
    device: &Object,
) -> PyResult<(Value, Value)> {
    let (voice, resource_id) = match profile {
        DeviceProfile::Legacy => (voice.to_owned(), resource_id.to_owned()),
        DeviceProfile::Upstream => (escape_xml(voice), escape_xml(resource_id)),
    };
    let babi = jv!({
        "feature_entrance": "editor",
        "feature_entrance_detail": "editor-feature-text_to_speech",
        "feature_key": "text_to_speech",
        "scenario": "video_editor",
    });
    let blocks: Vec<String> = texts
        .iter()
        .map(|t| {
            format!(
                "    <voice name=\"{voice}\" mock_tone_info=\"\" platform=\"sami\" resource_id=\"{resource_id}\" emotion=\"\" emotion_scale=\"0\" style=\"\" role=\"\" moyin_emotion=\"\" is_clone_tone=\"false\" need_subtitle_timestamp=\"false\">\n        <prosody rate=\"{rate}\">{}</prosody>\n    </voice>",
                escape_xml(t)
            )
        })
        .collect();
    let ssml = format!(
        "<speak version=\"1.0\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"en-US\">\n{}\n</speak>",
        blocks.join("\n")
    );
    let extra_info = "{\"benefit_info\":{}}";
    let sign = make_tts_payload_sign(
        &ssml,
        Some(extra_info),
        &dev_str(device, "device_id")?,
        &dev_str(device, "aid")?,
    )?;
    let payload = jv!({
        "audio_format": "mp3",
        "babi_param": dumps_compact(&babi),
        "credit_disable": false,
        "extra_info": extra_info,
        "need_merge_voice": false,
        "need_subtitle_timestamp": false,
        "scene": "text_to_speech",
        "ssml": ssml,
        "sign": sign,
    });
    let bind_id = new_uuid_lower();
    let context = new_uuid_lower();
    let body = jv!({
        "bind_id": bind_id,
        "can_queue": true,
        "enter_from": "text_to_speech",
        "tasks": [{
            "context": context,
            "payload": dumps_compact(&payload),
            "req_key": "sami_text_to_speech",
            "task_version": "v3",
        }],
    });
    Ok((babi, body))
}

#[must_use]
pub fn query_body(task_id: &str, token: &str, req_key: &str, bind_id: &str) -> Value {
    jv!({"tasks": [{"bind_id": bind_id, "id": task_id, "req_key": req_key, "task_version": "v3", "token": token}]})
}

/// What to send.
#[derive(Debug, Clone)]
pub enum Request<'a> {
    TtsNew {
        texts: &'a [String],
        voice: &'a str,
        resource_id: &'a str,
        rate: &'a str,
    },
    TtsQuery {
        task_id: &'a str,
        token: &'a str,
        bind_id: &'a str,
    },
}

/// A ready request: URL, headers in order (device values as they are), body text.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltRequest {
    pub url: String,
    pub headers: Vec<(String, Value)>,
    pub body: String,
}

impl BuiltRequest {
    /// Header values as strings, for the wire.
    #[must_use]
    pub fn string_headers(&self) -> Vec<(String, String)> {
        self.headers
            .iter()
            .map(|(k, v)| (k.clone(), str_of(v)))
            .collect()
    }
}

/// `build_request` for `tts-new` / `tts-query` with `device`.
///
/// # Errors
/// Missing device fields and RSA errors.
pub fn build_request(device: &Object, req: &Request<'_>) -> PyResult<BuiltRequest> {
    build_request_for(DeviceProfile::Legacy, device, req)
}

/// [`build_request`] with `profile`'s SSML rules.
///
/// # Errors
/// As [`build_request`].
pub fn build_request_for(
    profile: DeviceProfile,
    device: &Object,
    req: &Request<'_>,
) -> PyResult<BuiltRequest> {
    let (body, path, query) = match req {
        Request::TtsNew {
            texts,
            voice,
            resource_id,
            rate,
        } => {
            let (babi, body) = tts_new_body_for(profile, texts, voice, resource_id, rate, device)?;
            (
                body,
                "/lv/v1/common_task/new",
                common_query(device, Some(&babi), true)?,
            )
        }
        Request::TtsQuery {
            task_id,
            token,
            bind_id,
        } => (
            query_body(task_id, token, "sami_text_to_speech", bind_id),
            "/lv/v1/common_task/query",
            common_query(device, None, false)?,
        ),
    };
    let body_text = dumps_compact(&body);
    let url = format!("{BASE}{path}?{}", urlencode(&query));
    let mut headers = base_headers(device, &body_text, true)?;
    let device_time = headers
        .iter()
        .find(|(k, _)| k == "device-time")
        .map(|(_, v)| str_of(v))
        .unwrap_or_default();
    let sign = make_sign_header(
        &url,
        &dev_str(device, "appvr")?,
        &device_time,
        &dev_str(device, "tdid")?,
    );
    headers.push(("sign".into(), Value::from(sign)));
    Ok(BuiltRequest {
        url,
        headers,
        body: body_text,
    })
}
