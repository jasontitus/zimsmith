use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct IncomingLink {
    id: String,
    text: String,
    #[serde(default)]
    title: String,
}

#[derive(Clone, Serialize)]
pub struct SharedLink {
    pub id: String,
    pub url: Option<String>,
    pub title: String,
    pub error: Option<String>,
}

fn shared_url(text: &str) -> Result<String, String> {
    if text.len() > 32_768 {
        return Err("The shared text is too long. Share a single website link.".into());
    }
    let mut urls = Vec::new();
    for word in text.split_whitespace() {
        let word = word.trim_start_matches(['<', '(', '[', '"', '\'']);
        if !word.to_ascii_lowercase().starts_with("http://")
            && !word.to_ascii_lowercase().starts_with("https://")
        {
            continue;
        }
        let mut word = word.trim_end_matches(['>', '"', '\'']);
        for (open, close) in [('(', ')'), ('[', ']')] {
            while word.ends_with(close)
                && word.chars().filter(|c| *c == close).count()
                    > word.chars().filter(|c| *c == open).count()
            {
                word = &word[..word.len() - 1];
            }
        }
        let url = zim_engine::web::source_url(word)
            .map_err(|_| "Share a valid HTTP or HTTPS link without embedded credentials.")?;
        let url = url.to_string();
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    match urls.len() {
        1 => Ok(urls.remove(0)),
        0 => Err("No website link was found. Share a URL from your browser.".into()),
        _ => Err("Share one website link at a time.".into()),
    }
}

impl From<IncomingLink> for SharedLink {
    fn from(link: IncomingLink) -> Self {
        let mut title = String::new();
        for c in link.title.chars().filter(|c| !c.is_control()) {
            if title.len() + c.len_utf8() > 120 {
                break;
            }
            title.push(c);
        }
        match shared_url(&link.text) {
            Ok(url) => Self {
                id: link.id,
                url: Some(url.clone()),
                title: if title.trim().is_empty() {
                    reqwest::Url::parse(&url)
                        .ok()
                        .and_then(|u| u.host_str().map(str::to_owned))
                        .unwrap_or_else(|| "Shared website".into())
                } else {
                    title.trim().to_owned()
                },
                error: None,
            },
            Err(error) => Self {
                id: link.id,
                url: None,
                title: "Unusable shared link".into(),
                error: Some(error),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receives_browser_and_youtube_links_without_mistaking_shared_text_for_a_url() {
        assert_eq!(
            shared_url("A page title\nhttps://example.com/docs?q=one#top").unwrap(),
            "https://example.com/docs?q=one"
        );
        assert_eq!(
            shared_url("Watch this (https://youtu.be/fixture1234)").unwrap(),
            "https://youtu.be/fixture1234"
        );
        assert_eq!(
            shared_url("https://example.com/a_(b)").unwrap(),
            "https://example.com/a_(b)"
        );
        assert!(shared_url("Just some text").is_err());
        assert!(shared_url("file:///private/document").is_err());
        assert!(shared_url("https://user:password@example.com/").is_err());
        assert!(shared_url("https://example.com/ https://another.example/").is_err());
        assert!(shared_url(&"a".repeat(32_769)).is_err());
    }
}
