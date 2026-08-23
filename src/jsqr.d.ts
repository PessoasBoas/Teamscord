declare module "jsqr" {
  type InversionAttempts = "dontInvert" | "onlyInvert" | "attemptBoth";
  type QRCode = { data: string; location: unknown };
  export default function jsQR(data: Uint8ClampedArray, width: number, height: number, options?: { inversionAttempts?: InversionAttempts }): QRCode | null;
}
