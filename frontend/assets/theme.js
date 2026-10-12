// Apply a saved theme before the stylesheet and WASM load, including on login.
try {
  const theme = localStorage.getItem('causewaybay.theme.v1') === 'dark' ? 'dark' : 'light';
  document.documentElement.dataset.theme = theme;
  document.querySelector('meta[name="theme-color"]').content = theme === 'dark' ? '#18161e' : '#f8f7f4';
} catch {
  document.documentElement.dataset.theme = 'light';
}
