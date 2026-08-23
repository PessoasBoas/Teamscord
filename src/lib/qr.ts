import QRCode from "qrcode";
import jsQR from "jsqr";

export async function qrDataUrl(value: string, width = 280): Promise<string> {
  return QRCode.toDataURL(value, {
    width,
    margin: 2,
    errorCorrectionLevel: "M",
    color: { dark: "#171b32", light: "#ffffff" },
  });
}

export async function readQrImage(file: File): Promise<string> {
  const bitmap = await createImageBitmap(file);
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error("não foi possível preparar a leitura do QR");
  context.drawImage(bitmap, 0, 0);
  const image = context.getImageData(0, 0, canvas.width, canvas.height);
  const result = jsQR(image.data, image.width, image.height, { inversionAttempts: "attemptBoth" });
  if (!result?.data) throw new Error("nenhum QR válido foi encontrado na imagem");
  return result.data;
}
