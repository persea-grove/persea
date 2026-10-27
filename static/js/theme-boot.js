// Apply the stored theme before first paint (dark | light | auto).
// External script: the app's CSP allows 'self' but not un-nonced inline code,
// so this cannot live in a <script> block in the page.
(function () {
    try {
        var mode = localStorage.getItem('theme') || 'auto';
        var dark = mode === 'auto'
            ? window.matchMedia('(prefers-color-scheme: dark)').matches
            : mode === 'dark';
        document.documentElement.classList.add(dark ? 'dark' : 'light');
    } catch (e) {}
})();
