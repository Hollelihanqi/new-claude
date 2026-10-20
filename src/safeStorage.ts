// Accessing localStorage itself can throw under WebView policy restrictions.
export const browserStorage: Pick<Storage, "getItem" | "setItem"> = {
  getItem(key) {
    try { return window.localStorage.getItem(key); } catch { return null; }
  },
  setItem(key, value) {
    try { window.localStorage.setItem(key, value); } catch { /* Keep session preferences usable. */ }
  },
};
