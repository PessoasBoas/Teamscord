export const AUDIO_PREFERENCES_STORAGE = "teamscord.audio-preferences";
export const AUDIO_PREFERENCES_EVENT = "teamscord-audio-preferences";

export type AudioPreferences = {
  input_device_id: string;
  output_device_id: string;
  input_volume: number;
  output_volume: number;
};

export type AudioDevices = {
  inputs: MediaDeviceInfo[];
  outputs: MediaDeviceInfo[];
};

export const DEFAULT_AUDIO_PREFERENCES: AudioPreferences = {
  input_device_id: "",
  output_device_id: "",
  input_volume: 1,
  output_volume: 1,
};

function clamp(value: unknown, fallback: number): number {
  const number = typeof value === "number" && Number.isFinite(value) ? value : fallback;
  return Math.min(1, Math.max(0, number));
}

export function readAudioPreferences(): AudioPreferences {
  if (typeof window === "undefined") return DEFAULT_AUDIO_PREFERENCES;
  try {
    const stored = JSON.parse(localStorage.getItem(AUDIO_PREFERENCES_STORAGE) ?? "{}");
    return {
      input_device_id: typeof stored.input_device_id === "string" ? stored.input_device_id : "",
      output_device_id: typeof stored.output_device_id === "string" ? stored.output_device_id : "",
      input_volume: clamp(stored.input_volume, 1),
      output_volume: clamp(stored.output_volume, 1),
    };
  } catch {
    return DEFAULT_AUDIO_PREFERENCES;
  }
}

export function saveAudioPreferences(preferences: AudioPreferences): void {
  if (typeof window === "undefined") return;
  localStorage.setItem(AUDIO_PREFERENCES_STORAGE, JSON.stringify(preferences));
  window.dispatchEvent(new CustomEvent<AudioPreferences>(AUDIO_PREFERENCES_EVENT, { detail: preferences }));
}

export async function enumerateAudioDevices(): Promise<AudioDevices> {
  if (!navigator.mediaDevices?.enumerateDevices) return { inputs: [], outputs: [] };
  const devices = await navigator.mediaDevices.enumerateDevices();
  return {
    inputs: devices.filter((device) => device.kind === "audioinput"),
    outputs: devices.filter((device) => device.kind === "audiooutput"),
  };
}

export async function setOutputDevice(media: HTMLMediaElement, deviceId: string): Promise<void> {
  const sinkMedia = media as HTMLMediaElement & { setSinkId?: (sinkId: string) => Promise<void> };
  if (typeof sinkMedia.setSinkId === "function") await sinkMedia.setSinkId(deviceId);
}
