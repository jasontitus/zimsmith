(() => {
  for (const video of document.querySelectorAll("video[data-zim-poster]")) {
    const poster = video.dataset.zimPoster;
    // Artwork is an archive-local image, never a URL supplied by source scripts.
    if (!/^[a-z0-9_.-]+\.(?:jpe?g|png|webp)$/i.test(poster || "")) continue;
    const frame = document.createElement("div");
    frame.className = "zim-video-frame";
    const cover = document.createElement("button");
    cover.type = "button";
    cover.className = "zim-video-cover";
    const title = video.getAttribute("aria-label") || video.closest("figure")?.querySelector("figcaption")?.textContent || document.title;
    cover.setAttribute("aria-label", `Play ${title.trim()}`);
    const image = document.createElement("img");
    image.alt = "";
    image.addEventListener("error", () => cover.remove(), { once: true });
    image.src = poster;
    const play = document.createElement("span");
    play.textContent = "\u25b6";
    play.setAttribute("aria-hidden", "true");
    cover.append(image, play);
    video.replaceWith(frame);
    frame.append(video, cover);
    cover.hidden = !video.paused;
    video.addEventListener("play", () => { cover.hidden = true; });
    video.addEventListener("error", () => { cover.hidden = true; });
    cover.addEventListener("click", () => {
      cover.hidden = true;
      // Keep play() in the trusted tap for iOS media permission.
      video.play().catch(() => { cover.hidden = false; });
    });
  }
})();
