(() => {
  const player = document.getElementById("video-track");
  if (!player) return;
  function seek(play) {
    const match = location.hash.match(/^#t=(\d+(?:\.\d+)?)$/);
    if (!match) return;
    const time = Number(match[1]);
    if (!Number.isFinite(time)) return;
    const apply = () => {
      player.currentTime = Number.isFinite(player.duration) ? Math.min(time, Math.max(0, player.duration - 0.01)) : time;
      if (play) player.play().catch(() => {});
    };
    if (player.readyState) apply();
    else player.addEventListener("loadedmetadata", apply, { once: true });
  }
  seek(false);
  addEventListener("hashchange", () => seek(true));
  document.querySelectorAll(".transcript-cue a").forEach(link => link.addEventListener("click", event => {
    event.preventDefault();
    history.replaceState(null, "", link.getAttribute("href"));
    seek(true);
  }));
  const search = document.getElementById("transcript-search");
  const cues = Array.from(document.querySelectorAll(".transcript-cue"));
  search?.addEventListener("input", () => {
    const terms = search.value.toLocaleLowerCase().split(/\s+/).filter(Boolean);
    for (const cue of cues) cue.hidden = !terms.every(term => cue.textContent.toLocaleLowerCase().includes(term));
  });
})();
