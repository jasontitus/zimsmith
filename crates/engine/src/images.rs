//! Freeze common lazy/responsive image markup before its JavaScript is removed.
use anyhow::Result;
use lol_html::{element, rewrite_str, RewriteStrSettings};
use scraper::{Html, Selector};

fn candidates(value: &str) -> Vec<(String, f64)> {
    // A URL token ends at whitespace, not an arbitrary comma inside its path.
    let mut rest = value.trim();
    let mut candidates = Vec::new();
    while !rest.is_empty() && candidates.len() < 100 {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let token = &rest[..end];
        rest = rest[end..].trim_start();
        let (url, descriptor) = if token.ends_with(',') {
            (token.trim_end_matches(','), "")
        } else {
            let split = rest.find(',').unwrap_or(rest.len());
            let descriptor = rest[..split].trim();
            rest = rest[split..].trim_start_matches(',').trim_start();
            (token, descriptor)
        };
        if url.is_empty() || url.starts_with("data:") {
            continue;
        }
        let score = descriptor
            .strip_suffix('w')
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| {
                descriptor
                    .strip_suffix('x')
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|x| x * 800.0)
            })
            .unwrap_or(800.0);
        if score.is_finite() && score > 0.0 {
            candidates.push((url.to_owned(), score));
        }
    }
    candidates
}

fn srcset(value: &str) -> Option<String> {
    let mut candidates = candidates(value);
    candidates.sort_by(|a, b| a.1.total_cmp(&b.1));
    candidates
        .iter()
        .rev()
        .find(|(_, score)| *score <= 1600.0)
        .or_else(|| candidates.first())
        .map(|(url, _)| url.clone())
}

pub fn prepare(html: &str) -> Result<String> {
    let doc = Html::parse_document(html);
    let sources: Vec<_> = doc
        .select(&Selector::parse("img").unwrap())
        .map(|img| {
            let attr = |name| img.value().attr(name).filter(|s| !s.trim().is_empty());
            let picture = img
                .parent()
                .and_then(scraper::ElementRef::wrap)
                .filter(|parent| parent.value().name() == "picture");
            let selector = Selector::parse("source").unwrap();
            let sets: Vec<_> = picture
                .iter()
                .flat_map(|p| p.select(&selector))
                .flat_map(|s| {
                    [s.value().attr("data-srcset"), s.value().attr("srcset")]
                        .into_iter()
                        .flatten()
                })
                .collect();
            let source = ["data-src", "data-original", "data-lazy-src"]
                .into_iter()
                .find_map(attr)
                .map(str::to_owned)
                .or_else(|| {
                    attr("data-srcset")
                        .or_else(|| attr("srcset"))
                        .and_then(srcset)
                })
                .or_else(|| {
                    if attr("src").is_some_and(|s| !s.starts_with("data:")) {
                        return None;
                    }
                    sets.iter().copied().find_map(srcset)
                });
            let mut style = None;
            if let (Some(chosen), Some(original_style)) =
                (source.as_deref().or_else(|| attr("src")), attr("style"))
            {
                let variants: std::collections::HashSet<_> = sets
                    .into_iter()
                    .chain([attr("srcset"), attr("data-srcset")].into_iter().flatten())
                    .flat_map(candidates)
                    .map(|(url, _)| url)
                    .collect();
                if !variants.is_empty() && !chosen.starts_with("data:") {
                    let mut replacement = "url(".to_owned();
                    cssparser::serialize_string(chosen, &mut replacement).unwrap();
                    replacement.push(')');
                    let mut updated = original_style.to_owned();
                    for reference in crate::css::references(original_style).into_iter().rev() {
                        if variants.contains(&reference.value) && reference.value != chosen {
                            updated.replace_range(reference.start..reference.end, &replacement);
                        }
                    }
                    if updated != original_style {
                        style = Some(updated);
                    }
                }
            }
            (source, style)
        })
        .collect();
    let mut index = 0;
    let output = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("picture > source", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("img", |el| {
                    if let Some((source, style)) = sources.get(index) {
                        if let Some(style) = style {
                            el.set_attribute("style", style)?;
                        }
                        if let Some(source) = source {
                            if let Some(original) = el.get_attribute("src").filter(|s| s != source)
                            {
                                el.set_attribute("data-zim-original-src", &original)?;
                            }
                            el.set_attribute("src", source)?;
                        }
                    }
                    index += 1;
                    // Loading must not depend on scrolling, IntersectionObserver, or a script.
                    el.remove_attribute("loading");
                    Ok(())
                }),
            ],
            ..Default::default()
        },
    )?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn freezes_real_images_in_place_of_placeholders_and_bounds_resolution() {
        let html=prepare(r#"<img src="data:image/gif;base64,abc" data-src="real.jpg" loading="lazy"><picture><source srcset="small.webp 400w, large.webp 1200w, huge.webp 3000w"><img src="data:image/gif;base64,abc"></picture><img srcset="a,b.jpg 800w, large.jpg 2400w">"#).unwrap();
        let doc = Html::parse_document(&html);
        let images: Vec<_> = doc
            .select(&Selector::parse("img").unwrap())
            .map(|e| e.value().attr("src").unwrap())
            .collect();
        assert_eq!(images, ["real.jpg", "large.webp", "a,b.jpg"]);
        assert!(!html.contains("loading="));
    }
}
