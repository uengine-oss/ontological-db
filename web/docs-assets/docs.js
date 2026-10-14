// Documentation site: on-page contents, sidebar title filter.
(() => {
  const here = document.querySelector('.side a.here');
  if (here) here.scrollIntoView({ block: 'center' });

  // On this page: the h2 and h3 of the article.
  const toc = document.getElementById('toc');
  const heads = [...document.querySelectorAll('article h2[id], article h3[id]')];
  if (toc && heads.length > 1) {
    toc.innerHTML = '<b>이 페이지</b>' + heads.map((h) =>
      `<a class="${h.tagName.toLowerCase()}" href="#${h.id}">${h.textContent}</a>`).join('');
    const links = [...toc.querySelectorAll('a')];
    const mark = () => {
      let on = 0;
      heads.forEach((h, i) => { if (h.getBoundingClientRect().top < 90) on = i; });
      links.forEach((a, i) => a.classList.toggle('on', i === on));
    };
    addEventListener('scroll', mark, { passive: true });
    mark();
  }

  // Filter the sidebar by title; groups with a match open, the rest fold away.
  const box = document.getElementById('filter');
  const groups = [...document.querySelectorAll('.side details')];
  const wasOpen = groups.map((g) => g.open);
  box?.addEventListener('input', () => {
    const q = box.value.trim().toLowerCase();
    groups.forEach((g, i) => {
      let any = false;
      g.querySelectorAll('a').forEach((a) => {
        const hit = !q || a.textContent.toLowerCase().includes(q);
        a.classList.toggle('hidden', !hit);
        any ||= hit;
      });
      g.classList.toggle('hidden', !any);
      g.open = q ? any : wasOpen[i];
    });
  });
  addEventListener('keydown', (e) => {
    if (e.key === '/' && document.activeElement !== box) { e.preventDefault(); box.focus(); }
  });
})();
