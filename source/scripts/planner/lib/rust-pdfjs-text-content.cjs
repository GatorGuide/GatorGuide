const fs = require("fs");

async function main() {
  const inputPath = process.argv[2];
  if (!inputPath) {
    throw new Error("PDF input path is required");
  }

  const pdfjs = await import("pdfjs-dist/legacy/build/pdf.mjs");
  const data = new Uint8Array(await fs.promises.readFile(inputPath));
  const loadingTask = pdfjs.getDocument({
    data,
    disableFontFace: true,
    useSystemFonts: false,
    verbosity: 0,
  });
  let document = null;

  try {
    document = await loadingTask.promise;
    const pages = [];
    for (let pageNumber = 1; pageNumber <= document.numPages; pageNumber += 1) {
      const page = await document.getPage(pageNumber);
      try {
        const textContent = await page.getTextContent();
        const items = (textContent?.items ?? [])
          .filter((item) => typeof item?.str === "string")
          .map((item) => ({
            str: item.str,
            transform: Array.isArray(item.transform) ? item.transform : Array.from(item.transform ?? []),
            width: Number(item.width ?? 0),
            height: Number(item.height ?? 0),
          }));
        pages.push({ pageNumber, items });
      } finally {
        await page.cleanup?.();
      }
    }
    process.stdout.write(JSON.stringify(pages));
  } finally {
    if (document?.destroy) {
      await document.destroy();
    } else {
      await loadingTask.destroy?.();
    }
  }
}

main().catch((error) => {
  console.error(error?.stack ?? error);
  process.exitCode = 1;
});
