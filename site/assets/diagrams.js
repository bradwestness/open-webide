// Keep the code block readable if the optional renderer cannot load or parse it.
async function renderDiagrams() {
  const blocks = document.querySelectorAll('pre > code.language-mermaid');
  if (!blocks.length) return;
  try {
    const { default: mermaid } = await import('https://cdn.jsdelivr.net/npm/mermaid@11.12.0/dist/mermaid.esm.min.mjs');
    let layout = 'dagre';
    try {
      const { default: elk } = await import('https://cdn.jsdelivr.net/npm/@mermaid-js/layout-elk@0.1.9/dist/mermaid-layout-elk.esm.min.mjs');
      mermaid.registerLayoutLoaders(elk);
      layout = 'elk';
    } catch (error) {
      console.warn('ELK unavailable; using Mermaid default layout:', error);
    }
    mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', layout, theme: matchMedia('(prefers-color-scheme: light)').matches ? 'default' : 'dark' });
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

// A deferred classic script also works in downloaded/file:// previews, where
// browsers block the local module script before it can load the renderer.
renderDiagrams();
