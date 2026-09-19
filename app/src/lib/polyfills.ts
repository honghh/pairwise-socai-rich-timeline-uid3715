if (typeof Array.prototype.at !== "function") {
  Object.defineProperty(Array.prototype, "at", {
    configurable: true,
    writable: true,
    value<T>(this: T[], index: number): T | undefined {
      const relativeIndex = Math.trunc(index) || 0;
      const resolvedIndex = relativeIndex < 0 ? this.length + relativeIndex : relativeIndex;
      return resolvedIndex >= 0 && resolvedIndex < this.length ? this[resolvedIndex] : undefined;
    },
  });
}
