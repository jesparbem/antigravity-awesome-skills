import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

describe("renderMarkdown", () => {
  it("convierte el markdown que devuelve un modelo", () => {
    const html = renderMarkdown("Un **resumen** con `código`.\n\n- uno\n- dos");
    expect(html).toContain("<strong>resumen</strong>");
    expect(html).toContain("<code>código</code>");
    expect(html).toContain("<li>uno</li>");
  });

  it("elimina el script aunque la generación sea local", () => {
    const html = renderMarkdown('Hola <script>alert("x")</script> mundo');
    expect(html).not.toContain("<script");
    expect(html).toContain("Hola");
  });

  it("descarta atributos de evento", () => {
    expect(renderMarkdown('<p onclick="robar()">texto</p>')).not.toContain("onclick");
  });

  it("no convierte javascript: en un enlace vivo", () => {
    const html = renderMarkdown("[pulsa](javascript:alert(1))");
    expect(html).not.toContain("javascript:");
  });

  it("conserva los enlaces https", () => {
    expect(renderMarkdown("[naturgy](https://www.naturgy.com)")).toContain(
      'href="https://www.naturgy.com"',
    );
  });

  it("devuelve cadena vacía para texto vacío", () => {
    expect(renderMarkdown("")).toBe("");
  });
});
