/** The closed list of families, in the same order as `ai::TIPOS` in Rust. */
export const TIPOS = [
  "anime",
  "comic",
  "cartoon",
  "videojuego",
  "cine",
  "terror",
  "fantasia",
  "kawaii",
  "cotidiano",
  "frase",
  "otros",
] as const;

export type Tipo = (typeof TIPOS)[number];
