import { useEffect, useRef, useState } from "react";
import { Check, CircleAlert, Headphones, Mic, RefreshCw, Volume2, VolumeX } from "lucide-react";
import { enumerateAudioDevices, readAudioPreferences, saveAudioPreferences, type AudioDevices, type AudioPreferences } from "../lib/audio";

function deviceLabel(device: MediaDeviceInfo, fallback: string, index: number) {
  return device.label || `${fallback} ${index + 1}`;
}

export function AudioSettings() {
  const [preferences, setPreferences] = useState<AudioPreferences>(readAudioPreferences);
  const [devices, setDevices] = useState<AudioDevices>({ inputs: [], outputs: [] });
  const [status, setStatus] = useState("");
  const [level, setLevel] = useState(0);
  const cleanupRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let disposed = false;
    const refresh = async () => {
      try {
        const next = await enumerateAudioDevices();
        if (!disposed) setDevices(next);
      } catch (error) {
        if (!disposed) setStatus(`não foi possível listar dispositivos: ${String(error)}`);
      }
    };
    void refresh();
    navigator.mediaDevices?.addEventListener("devicechange", refresh);
    return () => {
      disposed = true;
      navigator.mediaDevices?.removeEventListener("devicechange", refresh);
      cleanupRef.current?.();
    };
  }, []);

  function update<K extends keyof AudioPreferences>(key: K, value: AudioPreferences[K]) {
    const next = { ...preferences, [key]: value };
    setPreferences(next);
    saveAudioPreferences(next);
  }

  function stopTest() {
    cleanupRef.current?.();
    cleanupRef.current = null;
    setLevel(0);
  }

  async function testInput() {
    stopTest();
    try {
      if (!navigator.mediaDevices?.getUserMedia) throw new Error("captura de microfone indisponível neste WebView2");
      const stream = await navigator.mediaDevices.getUserMedia({ audio: preferences.input_device_id ? { deviceId: { exact: preferences.input_device_id } } : true, video: false });
      const context = new AudioContext();
      const source = context.createMediaStreamSource(stream);
      const gain = context.createGain();
      const analyser = context.createAnalyser();
      const samples = new Uint8Array(analyser.fftSize);
      gain.gain.value = preferences.input_volume;
      source.connect(gain);
      gain.connect(analyser);
      let frame = 0;
      const readLevel = () => {
        analyser.getByteTimeDomainData(samples);
        const average = samples.reduce((total, sample) => total + Math.abs(sample - 128), 0) / samples.length;
        setLevel(Math.min(1, average / 34));
        frame = requestAnimationFrame(readLevel);
      };
      readLevel();
      setStatus("microfone funcionando — fale para testar o nível");
      cleanupRef.current = () => {
        cancelAnimationFrame(frame);
        stream.getTracks().forEach((track) => track.stop());
        void context.close();
        setStatus("");
      };
    } catch (error) {
      setStatus(`falha no microfone: ${String(error)}`);
    }
  }

  async function testOutput() {
    stopTest();
    try {
      const context = new AudioContext();
      const oscillator = context.createOscillator();
      const gain = context.createGain();
      const destination = context.createMediaStreamDestination();
      const audio = new Audio();
      gain.gain.value = Math.max(.02, preferences.output_volume * .12);
      oscillator.frequency.value = 660;
      oscillator.connect(gain);
      gain.connect(destination);
      audio.srcObject = destination.stream;
      audio.volume = preferences.output_volume;
      const sinkMedia = audio as HTMLAudioElement & { setSinkId?: (sinkId: string) => Promise<void> };
      if (preferences.output_device_id && typeof sinkMedia.setSinkId === "function") await sinkMedia.setSinkId(preferences.output_device_id);
      await audio.play();
      oscillator.start();
      setStatus("saída funcionando — tom de teste reproduzido");
      const timeout = window.setTimeout(() => stopTest(), 2_500);
      cleanupRef.current = () => {
        window.clearTimeout(timeout);
        oscillator.stop();
        audio.pause();
        audio.srcObject = null;
        void context.close();
        setStatus("");
      };
    } catch (error) {
      setStatus(`falha na saída de áudio: ${String(error)}`);
    }
  }

  return <div className="preference-form audio-settings">
    <div className="preference-section"><h3>Dispositivos de áudio</h3><p>Escolha onde o Teamscord captura sua voz e reproduz a call. Os nomes aparecem depois que o Windows libera a permissão do microfone.</p>
      <div className="audio-device-grid">
        <label className="audio-field"><span><Mic size={14} /> entrada</span><select value={preferences.input_device_id} onChange={(event) => update("input_device_id", event.target.value)}><option value="">microfone padrão</option>{devices.inputs.map((device, index) => <option key={device.deviceId} value={device.deviceId}>{deviceLabel(device, "microfone", index)}</option>)}</select></label>
        <label className="audio-field"><span><Headphones size={14} /> saída</span><select value={preferences.output_device_id} onChange={(event) => update("output_device_id", event.target.value)}><option value="">saída padrão</option>{devices.outputs.map((device, index) => <option key={device.deviceId} value={device.deviceId}>{deviceLabel(device, "saída", index)}</option>)}</select></label>
      </div>
    </div>
    <div className="preference-section audio-volume-section"><h3>Volume</h3><div className="volume-control"><span><Mic size={14} /> entrada <strong>{Math.round(preferences.input_volume * 100)}%</strong></span><input aria-label="Volume de entrada" type="range" min="0" max="1" step="0.01" value={preferences.input_volume} onChange={(event) => update("input_volume", Number(event.target.value))} /></div><div className="volume-control"><span>{preferences.output_volume === 0 ? <VolumeX size={14} /> : <Volume2 size={14} />} saída <strong>{Math.round(preferences.output_volume * 100)}%</strong></span><input aria-label="Volume de saída" type="range" min="0" max="1" step="0.01" value={preferences.output_volume} onChange={(event) => update("output_volume", Number(event.target.value))} /></div></div>
    <div className="audio-test-panel"><div><span className="eyebrow">DIAGNÓSTICO LOCAL</span><h3>Teste seus dispositivos</h3><p>O teste do microfone mede o sinal sem reproduzir sua voz para evitar microfonia. O teste de saída reproduz um tom curto.</p></div><div className="audio-test-actions"><button className="connect-button" onClick={() => void testInput()}><Mic size={14} /> testar microfone</button><button className="connect-button" onClick={() => void testOutput()}><Volume2 size={14} /> testar saída</button><button className="icon-button" onClick={() => void enumerateAudioDevices().then(setDevices)} title="Atualizar dispositivos" aria-label="Atualizar dispositivos"><RefreshCw size={15} /></button></div>{level > 0 && <div className="audio-meter" aria-label={`Nível do microfone ${Math.round(level * 100)}%`}><i style={{ transform: `scaleX(${Math.max(.04, level)})` }} /></div>}{status && <div className={`audio-test-status ${status.startsWith("falha") || status.startsWith("não") ? "error" : ""}`}>{status.startsWith("falha") || status.startsWith("não") ? <CircleAlert size={14} /> : <Check size={14} />} {status}</div>}</div>
  </div>;
}
