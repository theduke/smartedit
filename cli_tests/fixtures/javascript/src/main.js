/** A worker that runs normalized jobs. */
export class Worker {
  static version = "1.0";

  /** Create a worker. */
  constructor(name) {
    this.name = name;
  }

  /** Run one job asynchronously. */
  async run(job) {
    const normalize = (value) => String(value).trim();
    const callable = { call: (value) => normalize(value) };
    return callable.call(job);
  }
}

/** Public helpers. */
export const api = {
  greet(name) {
    return `hello ${name}`;
  },
  twice: (value) => value * 2,
  nested: {
    *values() {
      yield 1;
    },
  },
};

const first = 1; const second = 2;
export { first, second };
