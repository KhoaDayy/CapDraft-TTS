//! The fake CapCut `common_task` server of `rust/tools/fakecapcut.py`, for replays.

#![allow(dead_code, reason = "each test binary uses a different part")]
#![expect(clippy::unwrap_used, reason = "test helper")]

use capdraft_core::capcut_client::md5_hex;
use capdraft_core::json::{self as cj, Value, dumps_compact};
use capdraft_core::jv as json;
use capdraft_core::tts::{HttpResponse, Transport};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

static PROSODY: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"(?s)<prosody rate="[^"]*">(.*?)</prosody>"#).unwrap()
});
static CDN: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"https://cdn\.fake/(\w+)/(\d+)\.mp3").unwrap());

fn s(v: &Value) -> String {
    v.as_str().unwrap().to_owned()
}

#[derive(Default)]
pub struct FakeState {
    pub tasks: HashMap<String, (Vec<String>, u32)>,
    pub log: Vec<Value>,
}

#[derive(Default)]
pub struct FakeCapCut(pub Mutex<FakeState>);

pub fn resp(status: u16, body: &str) -> HttpResponse {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "",
    };
    HttpResponse {
        status,
        reason: reason.into(),
        body: body.as_bytes().to_vec(),
    }
}

impl Transport for FakeCapCut {
    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
        _t: Duration,
    ) -> Result<HttpResponse, String> {
        let mut st = self.0.lock().unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        let hs: Vec<Value> = headers.iter().map(|(k, v)| json!([k, v])).collect();
        st.log
            .push(json!({"method": "POST", "url": url, "headers": hs, "body": text.as_str()}));
        let req = cj::parse(&text).unwrap();
        if url.contains("/lv/v1/common_task/new") {
            let payload = cj::parse(req["tasks"][0]["payload"].as_str().unwrap()).unwrap();
            let ssml = s(&payload["ssml"]);
            let texts: Vec<String> = PROSODY
                .captures_iter(&ssml)
                .map(|c| c[1].to_owned())
                .collect();
            let joined = texts.concat();
            if joined.contains("#HTTP500") {
                return Ok(resp(500, "oops"));
            }
            if joined.contains("#RET1") {
                return Ok(resp(200, r#"{"ret":"1","errmsg":"bad request"}"#));
            }
            if joined.contains("#NOTASK") {
                return Ok(resp(200, r#"{"ret":"0","data":{"tasks":[]}}"#));
            }
            let tid = format!("T{}", &md5_hex(ssml.as_bytes())[..10]);
            st.tasks.insert(tid.clone(), (texts, 0));
            return Ok(resp(
                200,
                &dumps_compact(
                    &json!({"ret": "0", "data": {"tasks": [{"id": tid, "token": format!("K{tid}")}]}}),
                ),
            ));
        }
        let tid = s(&req["tasks"][0]["id"]);
        let t = st.tasks.get_mut(&tid).unwrap();
        t.1 += 1;
        let (texts, polls) = (t.0.clone(), t.1);
        let joined = texts.concat();
        if joined.contains("#QUERY500") && polls == 1 {
            return Ok(resp(500, "busy"));
        }
        if joined.contains("#QRET1") && polls == 3 {
            return Ok(resp(200, r#"{"ret":"5","errmsg":"poll broke"}"#));
        }
        if polls < 3 {
            return Ok(resp(
                200,
                &dumps_compact(
                    &json!({"ret": "0", "data": {"tasks": [{"id": tid, "status": "running"}]}}),
                ),
            ));
        }
        if joined.contains("#FAILTASK") {
            return Ok(resp(
                200,
                &dumps_compact(
                    &json!({"ret": "0", "data": {"tasks": [{"id": tid, "status": "failed"}]}}),
                ),
            ));
        }
        let mut subs: Vec<Value> = texts
            .iter()
            .enumerate()
            .map(|(i, tx)| {
                if tx.contains("#NOURL") {
                    json!({"duration": 1000})
                } else {
                    json!({"speech_url": format!("https://cdn.fake/{tid}/{i}.mp3"), "duration": 1000 + 37 * tx.chars().count() as i64})
                }
            })
            .collect();
        if joined.contains("#SHORT") {
            subs.pop();
        }
        let payload = dumps_compact(&json!({"audio_subtitles": subs}));
        Ok(resp(
            200,
            &dumps_compact(
                &json!({"ret": "0", "data": {"tasks": [{"id": tid, "status": "succeed", "payload": payload}]}}),
            ),
        ))
    }

    fn get(&self, url: &str, _t: Duration) -> Result<HttpResponse, String> {
        if url.starts_with("https://raw.githubusercontent.com/") {
            return Ok(resp(404, "")); // the voice catalog: not part of the transcript
        }
        let mut st = self.0.lock().unwrap();
        st.log.push(json!({"method": "GET", "url": url}));
        let c = CDN.captures(url).unwrap();
        let (tid, i) = (c[1].to_owned(), c[2].parse::<usize>().unwrap());
        let text = st.tasks[&tid].0[i].clone();
        if text.contains("#DLFAIL") {
            return Ok(resp(404, ""));
        }
        if text.contains("#DL500") {
            return Ok(resp(500, ""));
        }
        if text.contains("#EMPTY") {
            return Ok(resp(200, ""));
        }
        Ok(resp(200, &format!("MP3:{tid}:{i}:{text}")))
    }
}
