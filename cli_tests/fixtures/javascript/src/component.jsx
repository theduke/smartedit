/** A small badge component. */
export function Badge({ label }) {
  return <span className="badge">{label}</span>;
}

export const Panel = ({ children }) => (
  <section className="panel">{children}</section>
);
