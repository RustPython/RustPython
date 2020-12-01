export const getResponseTypeFromFetchType = (fetchEntry) => {
    if (fetchEntry === 'python') return 'text';
    if (fetchEntry === 'javascript') return 'text';
    if (fetchEntry === 'css') return 'text';
    if (fetchEntry === 'js') return 'blob';
    if (fetchEntry === 'plugin') return 'text';
    if (fetchEntry === 'bytes') return 'arrayBuffer';
    return fetchEntry;
};

export function genericFetch(path, fetchType) {
    const responseType = getResponseTypeFromFetchType(fetchType);
    return fetch(path)
        .then((r) => {
            if (!r.ok) throw new Error(`${r.status} ${r.statusText} (${path})`);
            return r[responseType]();
        })
        .then((r) => {
            if (fetchType === 'bytes') {
                return new Uint8Array(r);
            }
            return r;
        });
}
