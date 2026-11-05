//! Turn known script-dependent presentation states into static HTML.
use crate::archive::escape;
use anyhow::Result;
use lol_html::{element, rewrite_str, RewriteStrSettings};
use scraper::{Html, Selector};
use std::collections::{BTreeMap, BTreeSet};

pub fn reveal(class: &str, role: Option<&str>) -> bool {
    role.is_some()
        || class.split_whitespace().any(|c| {
            matches!(
                c,
                "transition--fade" | "transition--slide" | "transition--scale"
            )
        })
}

pub fn video_links(document: &Html) -> BTreeMap<String, String> {
    let mut links = BTreeMap::new();
    for video in document.select(&Selector::parse(".video[id]").unwrap()) {
        if video
            .select(&Selector::parse(".video__play--youtube").unwrap())
            .next()
            .is_none()
        {
            continue;
        }
        let Some(image) = video.select(&Selector::parse("img[src]").unwrap()).next() else {
            continue;
        };
        let Some(url) = image
            .value()
            .attr("src")
            .and_then(|s| crate::web::http_url(s).ok())
        else {
            continue;
        };
        if !matches!(url.host_str(), Some("i.ytimg.com" | "img.youtube.com")) {
            continue;
        }
        let segments: Vec<_> = url.path_segments().into_iter().flatten().collect();
        if segments.len() < 2 || !["vi", "vi_webp"].contains(&segments[0]) {
            continue;
        }
        let id = segments[1];
        if id.len() == 11
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            links.insert(
                video.value().attr("id").unwrap().to_string(),
                format!("https://youtu.be/{id}"),
            );
        }
    }
    links
}

pub fn prepare(html: &str, document: &Html) -> Result<String> {
    let dropdown = Selector::parse(".block-header-layout-mobile__dropdown").unwrap();
    let menu = document.select(&dropdown).next();
    let anchors = Selector::parse("a[href]").unwrap();
    let mut seen = BTreeSet::new();
    let links = menu
        .into_iter()
        .flat_map(|menu| menu.select(&anchors))
        .filter_map(|a| {
            let href = a.value().attr("href")?;
            let text = a
                .text()
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            (!text.is_empty() && seen.insert((href.to_owned(), text.clone())))
                .then(|| format!("<a href=\"{}\">{}</a>", escape(href), escape(&text)))
        })
        .take(100)
        .collect::<String>();
    let has_menu = !links.is_empty()
        && document
            .select(&Selector::parse("button.block-header__hamburger-menu").unwrap())
            .next()
            .is_some();
    let videos = video_links(document);
    let has_videos = !videos.is_empty();
    let catalogs = missing_catalogs(document);
    let loading_images = document
        .select(
            &Selector::parse("img.ecommerce-product-image--loading,img.lazyload,img.lazyloading")
                .unwrap(),
        )
        .next()
        .is_some();
    let product_images = document
        .select(&Selector::parse(".product-carousel__image-wrapper").unwrap())
        .next()
        .is_some();
    if !has_menu && !has_videos && catalogs.is_empty() && !loading_images && !product_images {
        return Ok(html.into());
    }
    // Some readers expand every <details> on load, including navigation menus.
    // This native checkbox keeps the menu closed and works without source JavaScript.
    let mut menu_index = 0;
    Ok(rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!(".product-carousel__image-wrapper", move |el| {
                    if product_images {
                        let classes = el.get_attribute("class").unwrap_or_default();
                        el.set_attribute("class", &format!("{classes} zim-static-product-image"))?;
                    }
                    Ok(())
                }),
                element!(".product-carousel__image-wrapper template", |el| {
                    el.remove_and_keep_content();
                    Ok(())
                }),
                element!("img", |el| {
                    if let Some(classes) = el.get_attribute("class") {
                        let lazy = classes
                            .split_whitespace()
                            .any(|c| ["lazyload", "lazyloading"].contains(&c));
                        let mut classes = classes
                            .split_whitespace()
                            .filter(|c| {
                                ![
                                    "ecommerce-product-image--loading",
                                    "lazyload",
                                    "lazyloading",
                                ]
                                .contains(c)
                            })
                            .collect::<Vec<_>>();
                        if lazy && !classes.contains(&"lazyloaded") {
                            classes.push("lazyloaded");
                        }
                        el.set_attribute("class", &classes.join(" "))?;
                    }
                    Ok(())
                }),
                element!(".block-product-list[id]", move |el| {
                    if el
                        .get_attribute("id")
                        .is_some_and(|id| catalogs.contains(&id))
                    {
                        el.set_inner_content("<aside class=\"zim-missing-content\"><h2>Store catalog not captured</h2><p>This catalog loads its products with JavaScript. Product listings are not included in this offline capture. Use Original source to browse the live store.</p></aside>",lol_html::html_content::ContentType::Html);
                    }
                    Ok(())
                }),
                element!("button.block-header__hamburger-menu", move |el| {
                    if has_menu {
                        let toggle = format!("zim-site-menu-toggle-{menu_index}");
                        menu_index += 1;
                        let replacement = format!("<div class=\"zim-site-menu\"><input class=\"zim-menu-toggle\" id=\"{toggle}\" type=\"checkbox\" aria-label=\"Site menu\"><label for=\"{toggle}\">Menu</label><nav aria-label=\"Site pages\">{links}</nav></div>");
                        el.replace(&replacement, lol_html::html_content::ContentType::Html);
                    }
                    Ok(())
                }),
                element!(".block-header-layout-mobile__dropdown", move |el| {
                    if has_menu {
                        el.remove();
                    }
                    Ok(())
                }),
                element!(".video[id]", move |el| {
                    if let Some(url) = el.get_attribute("id").and_then(|id| videos.get(&id)) {
                        el.append(&format!("<a class=\"zim-online-video\" href=\"{}\">Watch video online ↗</a>",escape(url)),lol_html::html_content::ContentType::Html);
                    }
                    Ok(())
                }),
                element!(".video__play--youtube", move |el| {
                    if has_videos {
                        el.remove();
                    }
                    Ok(())
                }),
            ],
            ..Default::default()
        },
    )?)
}

pub fn missing_catalogs(document: &Html) -> BTreeSet<String> {
    document
        .select(&Selector::parse(".block-product-list[id]").unwrap())
        .filter(|list| {
            // Category navigation is already present while the products are still loading.
            // It must not make an otherwise empty catalog count as captured.
            !list
                .select(&Selector::parse("a[href]").unwrap())
                .any(|link| {
                    !link
                        .value()
                        .classes()
                        .any(|class| class.starts_with("category-list__"))
                })
        })
        .filter_map(|list| list.value().attr("id").map(str::to_string))
        .collect()
}

pub fn needs_javascript(document: &Html) -> bool {
    if !missing_catalogs(document).is_empty() {
        return true;
    }
    if document
        .select(&Selector::parse("script").unwrap())
        .next()
        .is_none()
    {
        return false;
    }
    document
        .select(
            &Selector::parse("#app,#root,#__next,#__nuxt,#___gatsby,[data-reactroot],app-root")
                .unwrap(),
        )
        .any(|root| {
            let text = root.text().collect::<String>().trim().to_ascii_lowercase();
            text.is_empty()
                || matches!(
                    text.as_str(),
                    "loading" | "loading..." | "loading…" | "please wait" | "please wait..."
                )
        })
}
