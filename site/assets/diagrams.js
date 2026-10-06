// Keep the code block readable if the optional renderer cannot load or parse it.
const blocks = document.querySelectorAll('pre > code.language-mermaid');
if (blocks.length) {
  try {
    const { default: mermaid } = await import('https://cdn.jsdelivr.net/npm/mermaid@11.12.0/dist/mermaid.esm.min.mjs');
    mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: matchMedia('(prefers-color-scheme: light)').matches ? 'default' : 'dark' });
    for (const [index, block] of [...blocks].entries()) {
      try {
        const { svg } = await mermaid.render(`diagram-${index}`, block.textContent);
        const diagram = document.createElement('div');
        diagram.className = 'mermaid';
        diagram.innerHTML = svg;
        block.parentElement.replaceWith(diagram);
      } catch (error) {
        console.warn('Diagram kept as source:', error);
      }
    }
  } catch (error) {
    console.warn('Diagram renderer unavailable; keeping source:', error);
  }
}
