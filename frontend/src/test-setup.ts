// Node >= 26 exposes an undefined `localStorage` global when no
// --localstorage-file is configured. That prevents Vitest from copying the
// jsdom implementation onto globalThis, so wire the existing jsdom storage
// through explicitly instead of maintaining a second test-only implementation.
Object.defineProperty(globalThis, 'localStorage', {
  value: window.localStorage,
  writable: true,
  configurable: true,
});
