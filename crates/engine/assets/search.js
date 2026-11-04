(() => {
  const data = JSON.parse(document.getElementById("catalog").textContent);
  const input = document.getElementById("search");
  const cards = Array.from(document.querySelectorAll("[data-document]"));
  const count = document.getElementById("count");
  const empty = document.getElementById("empty-state");
  const clear = document.getElementById("clear-search");
  function search() {
    const terms = input.value.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean);
    let matches = null;
    for (const term of terms) {
      const postings = Object.prototype.hasOwnProperty.call(data.index, term) ? data.index[term] : [];
      const ids = new Set(Array.isArray(postings) ? postings : []);
      matches = matches === null ? ids : new Set([...matches].filter(id => ids.has(id)));
    }
    let visible = 0;
    for (const card of cards) {
      card.hidden = matches !== null && !matches.has(Number(card.dataset.document));
      card.querySelector(".transcript-match")?.remove();
      if (!card.hidden) {
        visible++;
        if (terms.length) {
          const item = data.documents[Number(card.dataset.document)];
          const cue = item.cues?.find(cue => terms.some(term => cue.text.toLowerCase().includes(term)));
          if (cue) {
            const link = document.createElement("a"); link.className = "transcript-match";
            link.href = `${item.url}#t=${cue.start}`;
            const seconds = Math.floor(cue.start);
            link.textContent = `Play at ${Math.floor(seconds/60)}:${String(seconds%60).padStart(2,"0")} · ${cue.text.slice(0,180)}`;
            card.append(link);
          }
        }
      }
    }
    count.textContent = terms.length ? `${visible} of ${data.documents.length} items` : `${data.documents.length} items to explore`;
    empty.hidden = visible > 0;
    clear.hidden = input.value.length === 0;
  }
  input.addEventListener("input", search);
  document.getElementById("search-form").addEventListener("submit", event => event.preventDefault());
  clear.addEventListener("click", () => { input.value = ""; search(); input.focus(); });
  input.addEventListener("keydown", event => { if (event.key === "Escape") { input.value = ""; search(); } });
})();
