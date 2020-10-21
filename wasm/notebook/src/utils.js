function injectJS(code) {
    const script = document.createElement('script');
    const doc = document.body || document.documentElement;
    const blob = new Blob([code], { type: 'text/javascript' });
    const url = URL.createObjectURL(blob);
    script.src = url;
    doc.appendChild(script);
    try {
      URL.revokeObjectURL(url);
      doc.removeChild(script);
    } catch (e) {
      // ignore if body is changed and script is detached
    }
  }

  export { injectJS }