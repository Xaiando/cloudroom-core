//! Text transport shared by Chat and the bounded Council protocol.
use super::*;

pub(super) async fn complete(profile: &Profile, messages: &[Value]) -> Result<(String, Value)> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|_| invalid("model transport unavailable"))?;
    let anthropic = profile.provider == "anthropic";
    let body = if anthropic {
        json!({"model":profile.model,"messages":messages,"max_tokens":8192,"stream":false})
    } else {
        json!({"model":profile.model,"messages":messages,"tool_choice":"none","stream":false})
    };
    let mut call = client.post(&profile.endpoint).json(&body);
    if anthropic {
        call = call
            .header("anthropic-version", "2023-06-01")
            .header("x-api-key", &profile.api_key);
    } else if !profile.api_key.is_empty() {
        call = call.bearer_auth(&profile.api_key);
    }
    let mut response = call
        .send()
        .await
        .map_err(|_| invalid("model request outcome uncertain"))?;
    if !response.status().is_success() {
        return Err(invalid(
            "model API rejected the request; check credentials and model access",
        ));
    }
    let mut bytes = vec![];
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| invalid("model response interrupted"))?
    {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err(invalid("model response exceeds 2 MiB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: Value =
        serde_json::from_slice(&bytes).map_err(|_| invalid("invalid model response"))?;
    if anthropic {
        let blocks = body["content"]
            .as_array()
            .ok_or(invalid("model returned no text"))?;
        if blocks.iter().any(|b| b["type"] != "text") {
            return Err(invalid(
                "unsupported model content; no provider tools executed",
            ));
        }
        let text = blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let input = body["usage"]["input_tokens"].as_u64().unwrap_or(0);
        let output = body["usage"]["output_tokens"].as_u64().unwrap_or(0);
        Ok((
            text,
            json!({"prompt_tokens":input,"completion_tokens":output,"total_tokens":input.saturating_add(output)}),
        ))
    } else {
        let message = &body["choices"][0]["message"];
        if message["tool_calls"]
            .as_array()
            .is_some_and(|c| !c.is_empty())
            || !message["function_call"].is_null()
        {
            return Err(invalid(
                "provider attempted an unsupported tool call; nothing executed",
            ));
        }
        let text = message["content"]
            .as_str()
            .ok_or(invalid("model returned no text"))?
            .to_owned();
        let mut usage = json!({});
        for key in ["prompt_tokens", "completion_tokens", "total_tokens"] {
            if let Some(n) = body["usage"][key].as_u64() {
                usage[key] = json!(n);
            }
        }
        Ok((text, usage))
    }
}

pub(super) fn qwen_endpoint(url: &reqwest::Url) -> bool {
    if url.scheme() != "https"
        || url.port().is_some()
        || url.path() != "/compatible-mode/v1/chat/completions"
    {
        return false;
    }
    let host = url.host_str().unwrap_or("");
    if [
        "dashscope.aliyuncs.com",
        "dashscope-intl.aliyuncs.com",
        "dashscope-us.aliyuncs.com",
        "cn-hongkong.dashscope.aliyuncs.com",
    ]
    .contains(&host)
    {
        return true;
    }
    let Some((workspace, region)) = host.split_once('.') else {
        return false;
    };
    !workspace.is_empty()
        && workspace
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && [
            "ap-southeast-1.maas.aliyuncs.com",
            "us-east-1.maas.aliyuncs.com",
            "cn-beijing.maas.aliyuncs.com",
            "cn-hongkong.maas.aliyuncs.com",
            "ap-northeast-1.maas.aliyuncs.com",
        ]
        .contains(&region)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qwen_hosts_are_exact_official_endpoints() {
        for endpoint in [
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions",
            "https://project-1.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/chat/completions",
        ] {
            assert!(qwen_endpoint(&reqwest::Url::parse(endpoint).unwrap()));
        }
        for endpoint in [
            "http://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions",
            "https://dashscope-intl.aliyuncs.com.evil.invalid/compatible-mode/v1/chat/completions",
            "https://project-1.cn-beijing.maas.aliyuncs.com/other",
            "https://evil.invalid/compatible-mode/v1/chat/completions",
        ] {
            assert!(!qwen_endpoint(&reqwest::Url::parse(endpoint).unwrap()));
        }
    }
}
