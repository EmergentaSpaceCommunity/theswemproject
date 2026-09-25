// Vite's `?raw` import: a file's exact text, which is how the shell inlines a
// dependency's stylesheet without serving a second file.
declare module "*?raw" {
  const contents: string;
  export default contents;
}
