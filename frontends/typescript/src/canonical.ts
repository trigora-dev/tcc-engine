/** Canonical JSON with sorted keys, matching `tcc_ir` BTreeMap stringify (no spaces). */
export function canonicalStringify(value: unknown): string {
  return write(value);
}

function write(value: unknown): string {
  if (value === null) {
    return "null";
  }
  if (value === undefined) {
    return "null";
  }
  if (typeof value === "boolean") {
    return value ? "true" : "false";
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      return "null";
    }
    return String(value);
  }
  if (typeof value === "string") {
    return writeString(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map((item) => write(item)).join(",")}]`;
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).sort(([a], [b]) =>
      a < b ? -1 : a > b ? 1 : 0,
    );
    return `{${entries.map(([key, item]) => `${writeString(key)}:${write(item)}`).join(",")}}`;
  }
  throw new TypeError("unsupported JSON value");
}

function writeString(text: string): string {
  let out = "\"";
  for (const ch of text) {
    switch (ch) {
      case "\"":
        out += "\\\"";
        break;
      case "\\":
        out += "\\\\";
        break;
      case "\n":
        out += "\\n";
        break;
      case "\r":
        out += "\\r";
        break;
      case "\t":
        out += "\\t";
        break;
      default:
        out += ch;
    }
  }
  return `${out}"`;
}
