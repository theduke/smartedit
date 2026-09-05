// TODO: remove compatibility marker
export interface Legacy {
  status: "draft";
}

export function title(value: string): string {
  return value.trim();
}
