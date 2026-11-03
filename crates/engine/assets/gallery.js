(() => {
  // Play controls are buttons: readers must not treat playback as a page link.
  const dialog = document.createElement("dialog");
  dialog.className = "gallery-player";
  dialog.setAttribute("aria-labelledby", "gallery-player-title");
  const header = document.createElement("div");
  header.className = "gallery-player-header";
  const title = document.createElement("h2");
  title.id = "gallery-player-title";
  const close = document.createElement("button");
  close.type = "button";
  close.textContent = "Back to gallery";
  close.setAttribute("aria-label", "Close player");
  const content = document.createElement("div");
  content.className = "gallery-player-content";
  const footer = document.createElement("div");
  footer.className = "gallery-player-footer";
  const queue = document.createElement("div");
  const position = document.createElement("p");
  position.className = "gallery-player-position";
  position.setAttribute("aria-live", "polite");
  const upcoming = document.createElement("p");
  upcoming.id = "gallery-up-next";
  upcoming.className = "gallery-up-next";
  const next = document.createElement("button");
  next.type = "button";
  next.textContent = "Next video →";
  next.setAttribute("aria-label", "Next video");
  next.setAttribute("aria-describedby", upcoming.id);
  const status = document.createElement("p");
  status.className = "gallery-player-status";
  status.setAttribute("role", "status");
  header.append(title, close);
  queue.append(position, upcoming);
  footer.append(queue, next);
  dialog.append(header, content, footer, status);
  document.body.append(dialog);
  let media = null;
  let trigger = null;
  let playlist = [];
  let currentIndex = -1;

  function stopMedia() {
    const previous = media;
    media = null;
    if (previous) {
      previous.pause();
      previous.removeAttribute("src");
      previous.load();
      previous.remove();
    }
  }
  function release() {
    stopMedia();
    document.body.classList.remove("gallery-playing");
    trigger?.focus({ preventScroll: true });
    trigger = null;
    playlist = [];
    currentIndex = -1;
  }
  function start(index) {
    const button = playlist[index];
    if (!button) return;
    stopMedia();
    currentIndex = index;
    trigger = button;
    title.textContent = button.dataset.playTitle;
    status.textContent = "";
    const audio = button.dataset.playKind === "Audio";
    const following = playlist[index + 1];
    position.textContent = `${audio ? "Track" : "Video"} ${index + 1} of ${playlist.length}`;
    next.textContent = audio ? "Next track →" : "Next video →";
    next.setAttribute("aria-label", audio ? "Next track" : "Next video");
    next.disabled = !following;
    upcoming.textContent = following
      ? `Up next: ${following.dataset.playTitle}`
      : "End of this collection";
    const current = document.createElement(audio ? "audio" : "video");
    media = current;
    current.controls = true;
    current.playsInline = true;
    current.preload = "metadata";
    current.setAttribute("aria-label", button.dataset.playTitle);
    current.src = button.dataset.playSrc;
    content.append(current);
    current.addEventListener("error", () => {
      if (media === current)
        status.textContent =
          "This video could not be played. Try its detail page from the gallery.";
    });
    current.addEventListener("ended", () => {
      if (media === current)
        status.textContent = following
          ? "Finished. Play the next video or return to the gallery."
          : "Finished. Return to the gallery to browse more.";
    });
    if (!dialog.open) dialog.showModal();
    document.body.classList.add("gallery-playing");
    // Keep play() in the trusted Play/Next click for iOS audio permission.
    current.play().catch((error) => {
      if (media === current)
        status.textContent =
          error.name === "NotAllowedError"
            ? "Tap Play in the player to start."
            : "Playback could not start. Try the player’s Play button.";
    });
  }
  close.addEventListener("click", () => dialog.close());
  next.addEventListener("click", () => start(currentIndex + 1));
  dialog.addEventListener("close", release);
  dialog.addEventListener("click", (event) => {
    if (event.target !== dialog) return;
    const bounds = dialog.getBoundingClientRect();
    if (
      event.clientX < bounds.left ||
      event.clientX > bounds.right ||
      event.clientY < bounds.top ||
      event.clientY > bounds.bottom
    )
      dialog.close();
  });
  document.addEventListener("contextmenu", (event) => {
    if (event.target.closest?.("button[data-play-src]")) event.preventDefault();
  });
  document.addEventListener("click", (event) => {
    const button = event.target.closest?.("button[data-play-src]");
    if (
      !button ||
      event.defaultPrevented ||
      event.button !== 0 ||
      typeof dialog.showModal !== "function"
    )
      return;
    event.preventDefault();
    playlist = Array.from(
      document.querySelectorAll("button[data-play-src]"),
    ).filter(
      (item) =>
        item.dataset.playKind === button.dataset.playKind &&
        !item.closest("[data-document]")?.hidden,
    );
    start(playlist.indexOf(button));
  });
})();
