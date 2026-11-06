//! Bounded sitemap discovery: optional, checkpointed, and under the normal network policy.
use crate::{web, PendingPage, Record};
use anyhow::{bail, Result};
use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SitemapState {
    pub initialized: bool,
    pub queue: Vec<String>,
    pub checked: Vec<String>,
    #[serde(default)]
    pub limited: bool,
}

fn parse(xml: &str) -> Result<(bool, Vec<String>)> {
    anyhow::ensure!(
        xml.len() <= 2 * 1024 * 1024,
        "Sitemap exceeds 2 MB parsing limit"
    );
    let mut reader = Reader::from_str(xml);
    let mut index = false;
    let mut recognized = false;
    let mut inside = false;
    let mut text = String::new();
    let mut urls = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                if !recognized {
                    anyhow::ensure!(
                        ["urlset", "sitemapindex"].contains(&e.local_name().as_ref()),
                        "Response is not a sitemap XML document"
                    );
                    recognized = true;
                }
                match e.local_name().as_ref() {
                    "sitemapindex" => index = true,
                    "loc" => {
                        inside = true;
                        text.clear();
                    }
                    _ => (),
                }
            }
            Event::Empty(e) if !recognized => {
                anyhow::ensure!(
                    ["urlset", "sitemapindex"].contains(&e.local_name().as_ref()),
                    "Response is not a sitemap XML document"
                );
                recognized = true;
                index = e.local_name().as_ref() == "sitemapindex";
            }
            Event::Text(e) if inside => text.push_str(&e.xml10_content()),
            Event::CData(e) if inside => text.push_str(&e.xml10_content()),
            Event::GeneralRef(e) if inside => {
                let reference = format!("&{};", e.xml10_content());
                text.push_str(&quick_xml::escape::unescape(&reference)?);
            }
            Event::End(e) if e.local_name().as_ref() == "loc" => {
                inside = false;
                if urls.len() < 50_000 {
                    urls.push(text.trim().to_string());
                }
            }
            Event::DocType(_) => bail!("Sitemap document types are unsupported"),
            Event::Eof => break,
            _ => (),
        }
    }
    anyhow::ensure!(recognized, "Response is not a sitemap XML document");
    Ok((index, urls))
}

pub async fn discover(record: &Record) -> Result<()> {
    {
        let j = record.job.lock().await;
        if !j.config.capture.discover_sitemaps || j.config.max_depth == 0 {
            return Ok(());
        }
    }
    record
        .change(|j| {
            if !j.sitemap_checkpoint.initialized {
                j.sitemap_checkpoint.initialized = true;
                for seed in j.config.starting_pages() {
                    if let Ok(url) = web::http_url(&seed.url)
                        .and_then(|u| u.join("/sitemap.xml").map_err(Into::into))
                    {
                        if j.sitemap_checkpoint.queue.len() >= 8 {
                            j.sitemap_checkpoint.limited = true;
                            continue;
                        }
                        if !j.sitemap_checkpoint.queue.contains(&url.to_string()) {
                            j.sitemap_checkpoint.queue.push(url.to_string());
                        }
                    }
                }
            }
        })
        .await?;
    let network = web::Network::new(record).await?.with_cap(2 * 1024 * 1024);
    loop {
        let snapshot = record.job.lock().await.clone();
        let state = &snapshot.sitemap_checkpoint;
        if state.checked.len() >= 8
            || snapshot.queue.len() + snapshot.visited.len() >= snapshot.config.max_items * 10
        {
            break;
        }
        let Some(value) = state.queue.first().cloned() else {
            break;
        };
        let url = web::http_url(&value)?;
        let result = network.get_same_origin(&url).await;
        // robots.txt was checked for this origin by the normal request path.
        let declared = network.declared_sitemaps(&url).await;
        let parsed = result.and_then(|(effective, _, bytes)| {
            anyhow::ensure!(
                effective.origin() == url.origin(),
                "Cross-origin sitemap redirect ignored"
            );
            parse(std::str::from_utf8(&bytes)?)
        });
        record.change(|j| {
            j.sitemap_checkpoint.queue.remove(0);
            j.sitemap_checkpoint.checked.push(value.clone());
            let mut maps = declared;
            match parsed {
                Ok((index, urls)) => {
                    if index { maps.extend(urls); }
                    else {
                        let mut added=0;
                        for value in urls {
                            if j.queue.len() + j.visited.len() >= j.config.max_items * 10 { j.scan_truncated = true; break; }
                            if let Some(candidate) = web::resolve(&url, &value) {
                                let value = candidate.to_string();
                                if j.config.contains_url(&candidate) && j.config.page_allowed(candidate.as_str()) && !crate::capture::linked_asset(&candidate)
                                    && !j.visited.contains(&value) && !j.queue.iter().any(|p| p.url == value) {
                                    j.queue.push(PendingPage { url: value, depth: 1 }); added+=1;
                                }
                            }
                        }
                        j.log("info", format!("Sitemap added {added} in-scope page URLs: {value}"));
                    }
                },
                Err(error) => j.log("warning", format!("Sitemap discovery skipped {value}: {error:#}. Ordinary page discovery continues.")),
            }
            for map in maps {
                let Some(map) = web::resolve(&url, &map).filter(|u| u.origin() == url.origin()).map(|u| u.to_string()) else { continue; };
                if !j.sitemap_checkpoint.queue.contains(&map) && !j.sitemap_checkpoint.checked.contains(&map) {
                    if j.sitemap_checkpoint.queue.len() + j.sitemap_checkpoint.checked.len() >= 8 { j.sitemap_checkpoint.limited = true; break; }
                    j.sitemap_checkpoint.queue.push(map);
                }
            }
        }).await?;
    }
    record.change(|j| {
        if j.sitemap_checkpoint.limited { j.scan_truncated = true; j.log("warning", "Sitemap discovery reached its 8-file limit. Some pages may be missing; review the scan before downloading."); }
    }).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_namespaces_entities_indexes_and_rejects_doctypes() {
        let (index, urls)=parse("<s:urlset xmlns:s='urn:s'><s:url><s:loc>https://example.test/?a=1&amp;b=2</s:loc></s:url></s:urlset>").unwrap();
        assert!(parse("<html><body>Not a sitemap</body></html>").is_err());
        assert!(!index);
        assert_eq!(urls, ["https://example.test/?a=1&b=2"]);
        assert!(parse("<sitemapindex><sitemap><loc>https://example.test/one.xml</loc></sitemap></sitemapindex>").unwrap().0);
        assert!(
            parse("<!DOCTYPE foo [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><urlset/>").is_err()
        );
    }
}
