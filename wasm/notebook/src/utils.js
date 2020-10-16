export const getResponseTypeFromFetchType = fetchEntry => {
    if (fetchEntry === "python") return "text";
    if (fetchEntry === "javascript") return "text";
    if (fetchEntry === "css") return "text";
    if (fetchEntry === "js") return "blob";
    if (fetchEntry === "plugin") return "text";
    if (fetchEntry === "bytes") return "arrayBuffer";
    return fetchEntry;
  };
  
  export function genericFetch(path, fetchType) {
    const responseType = getResponseTypeFromFetchType(fetchType);
    return fetch(path)
      .then(r => {
        if (!r.ok) throw new Error(`${r.status} ${r.statusText} (${path})`);
        return r[responseType]();
      })
      .then(r => {
        if (fetchType === "bytes") {
          return new Uint8Array(r);
        }
        return r;
      });
  }
  
  export function inject(code) {
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