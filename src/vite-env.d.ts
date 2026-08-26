/// <reference types="vite/client" />

// Lets `import "@/index.css"` typecheck as a side-effect import.
declare module "*.css" {
  const content: string;
  export default content;
}
