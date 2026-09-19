const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

test('reel cover remains an image when no playable video URL is available', () => {
  const coverUrl = 'https://scontent.cdninstagram.com/reel-cover.jpg?token=fixture';
  const image = {
    tagName: 'IMG',
    src: coverUrl,
    currentSrc: coverUrl,
    alt: 'Reel cover',
    closest: () => null,
  };
  const article = {
    querySelectorAll: (selector) => selector === 'video, img[src]' ? [image] : [],
  };
  const main = {
    querySelector: (selector) => selector === 'article' ? article : null,
    querySelectorAll: (selector) => selector === 'article, [role="dialog"]' ? [article] : [],
  };
  const metadata = {
    'og:type': 'article',
    'og:url': 'https://www.instagram.com/reel/Fixture123/',
    'og:image': coverUrl,
    description: '10 likes, 2 comments - author on September 17, 2026: “Fixture caption”.',
  };
  const document = {
    body: { innerText: 'Fixture reel page with hydrated content' },
    readyState: 'complete',
    title: 'Fixture reel',
    querySelector: (selector) => {
      const meta = selector.match(/^meta\[property="([^"]+)"\], meta\[name="\1"\]$/);
      if (meta && metadata[meta[1]]) return { getAttribute: () => metadata[meta[1]] };
      if (selector === 'main') return main;
      if (selector === 'main img[src], main video') return image;
      return null;
    },
    querySelectorAll: () => [],
  };
  const window = { getComputedStyle: () => ({ visibility: 'visible', display: 'block' }) };
  const context = {
    URL,
    document,
    location: {
      href: 'https://www.instagram.com/p/Fixture123/',
      pathname: '/p/Fixture123/',
    },
    window,
  };
  const source = fs.readFileSync(path.join(__dirname, 'page_scripts.js'), 'utf8');
  vm.runInNewContext(source, context);

  const detail = window.SocaiInstagramPageScripts.postDetail();

  assert.equal(detail.ok, true);
  assert.equal(detail.kind, 'reel');
  assert.equal(detail.media.length, 1);
  assert.equal(detail.media[0].type, 'image');
  assert.equal(detail.media[0].url, coverUrl);
  assert.equal(detail.media[0].poster_url, '');
  assert.equal(typeof window.SocaiInstagramPageScripts.scrollComments, 'function');
});

test('post detail exposes the playable Instagram video URL', () => {
  const videoUrl = 'https://scontent.cdninstagram.com/o1/v/t16/fixture.mp4?token=signed';
  const metadata = {
    'og:type': 'article',
    'og:url': 'https://www.instagram.com/reel/Video123/',
    'og:video': videoUrl,
    description: '12 likes, 3 comments - creator on September 18, 2026: “Video caption”.',
  };
  const main = {
    querySelector: () => null,
    querySelectorAll: () => [],
  };
  const document = {
    body: { innerText: 'Hydrated Instagram reel' },
    readyState: 'complete',
    title: 'Video fixture',
    querySelector: (selector) => {
      const meta = selector.match(/^meta\[property="([^"]+)"\], meta\[name="\1"\]$/);
      if (meta && metadata[meta[1]]) return { getAttribute: () => metadata[meta[1]] };
      if (selector === 'main') return main;
      return null;
    },
    querySelectorAll: () => [],
  };
  const window = { getComputedStyle: () => ({ visibility: 'visible', display: 'block' }) };
  const context = {
    URL,
    document,
    location: {
      href: 'https://www.instagram.com/reel/Video123/',
      pathname: '/reel/Video123/',
    },
    performance: { getEntriesByType: () => [] },
    window,
  };
  const source = fs.readFileSync(path.join(__dirname, 'page_scripts.js'), 'utf8');
  vm.runInNewContext(source, context);

  const detail = window.SocaiInstagramPageScripts.postDetail();

  assert.equal(detail.ok, true);
  assert.equal(detail.media.length, 1);
  assert.equal(detail.media[0].type, 'video');
  assert.equal(detail.media[0].url, videoUrl);
});

test('comments retain visible replies as a nested tree', () => {
  const body = { parentElement: null };
  const list = {
    parentElement: body,
    matches: (selector) => selector.includes('ul'),
  };
  function fixtureComment(id, author, text, relative, left) {
    const authorLink = { href: `https://www.instagram.com/${author}/` };
    const commentLink = { href: `https://www.instagram.com/p/Post123/c/${id}/` };
    const row = {
      innerText: `${author}\n${text}\n${relative}`,
      parentElement: list,
      querySelectorAll: (selector) => selector === 'a[href]' ? [authorLink] : [],
      getBoundingClientRect: () => ({ left }),
      matches: () => false,
    };
    const time = {
      innerText: relative,
      dateTime: `2026-09-18T0${id}:00:00Z`,
      parentElement: row,
      closest: (selector) => selector === 'a[href*="/c/"]' ? commentLink : null,
      getAttribute: () => '',
    };
    return time;
  }
  const times = [
    fixtureComment('1', 'parent_user', 'Parent comment', '1h', 120),
    fixtureComment('2', 'reply_user', 'Nested reply', '30m', 156),
  ];
  const document = {
    body,
    querySelectorAll: (selector) => selector === 'a[href*="/c/"] time[datetime]' ? times : [],
    querySelector: () => null,
  };
  const window = { getComputedStyle: () => ({ visibility: 'visible', display: 'block' }) };
  const context = {
    URL,
    document,
    location: {
      href: 'https://www.instagram.com/p/Post123/',
      pathname: '/p/Post123/',
    },
    window,
  };
  const source = fs.readFileSync(path.join(__dirname, 'page_scripts.js'), 'utf8');
  vm.runInNewContext(source, context);

  const comments = window.SocaiInstagramPageScripts.comments({ limit: 10 });

  assert.equal(comments.length, 1);
  assert.equal(comments[0].id, '1');
  assert.equal(comments[0].replies.length, 1);
  assert.equal(comments[0].replies[0].id, '2');
  assert.equal(comments[0].replies[0].text, 'Nested reply');
});
