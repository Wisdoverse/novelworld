// Node >= 26 ships a native `localStorage` global (undefined without
// --localstorage-file) that exists on globalThis, so vitest's jsdom
// environment skips populating it and every suite touching the token
// storage fails with 'undefined'. The app only uses the synchronous
// Storage surface, so provide an in-memory implementation for the test
// environment.
class MemoryStorage implements Storage {
  private store = new Map<string, string>();

  get length(): number {
    return this.store.size;
  }

  clear(): void {
    this.store.clear();
  }

  getItem(key: string): string | null {
    return this.store.has(key) ? (this.store.get(key) ?? null) : null;
  }

  key(index: number): string | null {
    return Array.from(this.store.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.store.delete(key);
  }

  setItem(key: string, value: string): void {
    this.store.set(key, String(value));
  }
}

Object.defineProperty(globalThis, 'localStorage', {
  value: new MemoryStorage(),
  writable: true,
  configurable: true,
});
