/**
 * Markdown de las respuestas.
 *
 * El texto lo produce un modelo, así que se sanea siempre antes de insertarlo
 * en el documento — aunque la generación sea local.
 */

import { marked } from "marked";
import DOMPurify from "dompurify";

marked.setOptions({ breaks: true, gfm: true });

export function renderMarkdown(text: string): string {
  const html = marked.parse(text, { async: false }) as string;
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS: [
      "p", "br", "strong", "em", "del", "code", "pre", "blockquote",
      "ul", "ol", "li", "h1", "h2", "h3", "h4", "h5", "h6",
      "table", "thead", "tbody", "tr", "th", "td", "hr", "a",
    ],
    ALLOWED_ATTR: ["href", "title"],
    // Un modelo puede escribir `javascript:`; nunca se convierte en enlace vivo.
    ALLOWED_URI_REGEXP: /^(?:https?|mailto):/i,
  });
}
