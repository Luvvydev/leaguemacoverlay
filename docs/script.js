const tabs = document.querySelectorAll('.feature-tab');
const image = document.querySelector('#feature-image');
const kicker = document.querySelector('#feature-kicker');
const title = document.querySelector('#feature-title');
const copy = document.querySelector('#feature-copy');

tabs.forEach((tab) => {
  tab.addEventListener('click', () => {
    tabs.forEach((item) => {
      item.classList.remove('active');
      item.setAttribute('aria-selected', 'false');
    });
    tab.classList.add('active');
    tab.setAttribute('aria-selected', 'true');
    image.style.opacity = '0';
    setTimeout(() => {
      image.src = tab.dataset.image;
      image.alt = tab.dataset.alt;
      kicker.textContent = tab.dataset.kicker;
      title.textContent = tab.dataset.title;
      copy.textContent = tab.dataset.copy;
      image.style.opacity = '1';
    }, 160);
  });
});

image.style.transition = 'opacity .2s ease';

const observer = new IntersectionObserver((entries) => {
  entries.forEach((entry) => {
    if (entry.isIntersecting) {
      entry.target.classList.add('visible');
      observer.unobserve(entry.target);
    }
  });
}, { threshold: 0.12 });

document.querySelectorAll('.reveal').forEach((element) => observer.observe(element));

fetch('https://api.github.com/repos/Luvvydev/leaguemacoverlay/releases/latest')
  .then((response) => {
    if (!response.ok) throw new Error('Release lookup failed');
    return response.json();
  })
  .then((release) => {
    const mac = release.assets.find((asset) => asset.name.endsWith('_aarch64.dmg'));
    const windows = release.assets.find((asset) => asset.name.endsWith('_x64-setup.exe'));
    if (mac) document.querySelectorAll('.mac-download').forEach((link) => { link.href = mac.browser_download_url; });
    if (windows) document.querySelectorAll('.windows-download').forEach((link) => { link.href = windows.browser_download_url; });
    document.querySelectorAll('.current-version').forEach((label) => { label.textContent = release.name || release.tag_name; });
  })
  .catch(() => {});
