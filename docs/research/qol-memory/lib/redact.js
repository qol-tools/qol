export function redact(text) {
  if (typeof text !== "string" || !text) return text;
  return text
    .replace(/\b[A-Za-z0-9_\-]{32,}\b/g, "[REDACTED]")
    .replace(/(?:Bearer|Token|api[_-]?key|password|passwd|secret|private[_-]?key)\s*[:=]\s*[\S]+/gi, "$1=[REDACTED]")
    .replace(/sk-[A-Za-z0-9]{20,}/g, "[REDACTED-KEY]")
    .replace(/-----BEGIN[\s\S]*?END [A-Z ]*-----/g, "[REDACTED-PEM]")
    .replace(/([\w.+-]+@[\w.-]+\.\w{2,})/g, "[EMAIL]")
    .replace(/\.env[\s\S]*/g, ".env [REDACTED]");
}
