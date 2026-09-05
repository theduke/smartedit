/** Persistent storage used by the service. */
export interface Repository<T> {
  readonly name: string;
  find(id: string): Promise<T | undefined>;
  save(value: T): Promise<void>;
}

/** Starts and monitors work. */
@injectable
export abstract class Service<T extends { id: string }> {
  protected abstract readonly repository: Repository<T>;
  public readonly invoke: (id: string) => Promise<T>;

  constructor(public readonly label: string) {
    this.invoke = (id) => this.run(id);
  }

  abstract status(): "ready" | "stopped";

  /** Find one item or fail. */
  async run(id: string): Promise<T> {
    const item = await this.repository.find(id);
    if (!item) throw new Error(`missing ${id}`);
    return item;
  }
}

export type ServiceId = `${string}:${number}`;

export const enum Priority {
  Low = 1,
  High = 2,
}

export function format(value: string): string;
export function format(value: number): string;
export function format(value: string | number): string {
  return String(value).trim();
}
