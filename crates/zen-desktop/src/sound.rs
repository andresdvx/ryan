//! Sonidos de la mascota. Se sintetizan al vuelo (unas notas suaves) y se reproducen desde memoria:
//! no hay archivos de audio que instalar y el volumen es el de la propia señal, sin tocar el del sistema.

use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};

const SAMPLE_RATE: u32 = 22_050;
/// A volumen 100 la señal llega a esta fracción del máximo: es un aviso, no una alarma de incendios.
const PEAK: f32 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundKind {
    /// Venció un recordatorio.
    Reminder,
    /// Algo necesita tu atención (confirmación pendiente, batería baja…).
    Notify,
    /// Una tarea terminó bien.
    Success,
    /// Algo salió mal.
    Error,
}

impl SoundKind {
    /// Notas (hercios, milisegundos) de cada sonido.
    fn notes(self) -> &'static [(f32, u32)] {
        match self {
            SoundKind::Reminder => &[(784.0, 140), (988.0, 140), (1175.0, 140), (988.0, 140), (1175.0, 260)],
            SoundKind::Notify => &[(880.0, 120), (1175.0, 200)],
            SoundKind::Success => &[(659.0, 90), (880.0, 160)],
            SoundKind::Error => &[(392.0, 150), (294.0, 280)],
        }
    }
}

/// Volumen en porcentaje, limitado a 0..=100.
pub fn clamp_volume(volume: u32) -> u8 {
    volume.min(100) as u8
}

/// El sonido como archivo WAV (16 bits, mono) con la amplitud que corresponde al volumen.
pub fn synthesize(kind: SoundKind, volume: u8) -> Vec<u8> {
    let gain = PEAK * (volume.min(100) as f32 / 100.0);
    let mut samples: Vec<i16> = Vec::new();
    for &(frequency, millis) in kind.notes() {
        let count = (SAMPLE_RATE as u64 * millis as u64 / 1000) as usize;
        for index in 0..count {
            let t = index as f32 / SAMPLE_RATE as f32;
            // Entrada y salida suaves de cada nota para que no haya «clics».
            let fade = (index.min(count - index) as f32 / (SAMPLE_RATE as f32 * 0.012)).min(1.0);
            let wave = (std::f32::consts::TAU * frequency * t).sin() + 0.25 * (std::f32::consts::TAU * 2.0 * frequency * t).sin();
            samples.push((wave / 1.25 * fade * gain * i16::MAX as f32) as i16);
        }
    }
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

/// Reproduce el sonido en segundo plano. Con volumen 0 no hace nada.
pub fn play(kind: SoundKind, volume: u8) {
    if volume == 0 {
        return;
    }
    let wav = synthesize(kind, volume);
    std::thread::spawn(move || {
        // SAFETY: con SND_MEMORY el puntero es el WAV completo; SND_SYNC bloquea este hilo hasta terminar,
        // así que `wav` sigue vivo mientras Windows lo lee.
        unsafe {
            PlaySoundW(wav.as_ptr() as *const u16, std::ptr::null_mut(), SND_MEMORY | SND_NODEFAULT | SND_SYNC);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(wav: &[u8]) -> i16 {
        wav[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]).saturating_abs()).max().unwrap_or(0)
    }

    #[test]
    fn el_wav_tiene_una_cabecera_valida_y_la_duracion_de_las_notas() {
        for kind in [SoundKind::Reminder, SoundKind::Notify, SoundKind::Success, SoundKind::Error] {
            let wav = synthesize(kind, 50);
            assert_eq!(&wav[0..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            assert_eq!(&wav[36..40], b"data");
            let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
            assert_eq!(data_len, wav.len() - 44);
            assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize, wav.len() - 8);
            let millis: u32 = kind.notes().iter().map(|n| n.1).sum();
            let expected = (kind.notes().iter().map(|&(_, ms)| SAMPLE_RATE as u64 * ms as u64 / 1000).sum::<u64>() * 2) as usize;
            assert_eq!(data_len, expected, "{kind:?} dura {millis} ms");
        }
    }

    #[test]
    fn el_volumen_escala_la_amplitud() {
        let silent = synthesize(SoundKind::Notify, 0);
        let low = synthesize(SoundKind::Notify, 25);
        let high = synthesize(SoundKind::Notify, 100);
        assert_eq!(peak(&silent), 0);
        assert!(peak(&low) > 0 && peak(&low) < peak(&high), "{} {}", peak(&low), peak(&high));
        assert!((peak(&high) as f32) <= i16::MAX as f32 * PEAK + 1.0, "nunca llega al máximo");
        let ratio = peak(&low) as f32 / peak(&high) as f32;
        assert!((ratio - 0.25).abs() < 0.03, "{ratio}");
        assert_eq!(synthesize(SoundKind::Notify, 250), high, "más de 100 se trata como 100");
    }

    #[test]
    fn el_volumen_se_limita_a_cien() {
        assert_eq!((clamp_volume(0), clamp_volume(70), clamp_volume(100), clamp_volume(101), clamp_volume(u32::MAX)), (0, 70, 100, 100, 100));
    }

    #[test]
    fn las_notas_empiezan_y_acaban_en_silencio() {
        let wav = synthesize(SoundKind::Error, 100);
        let first = i16::from_le_bytes([wav[44], wav[45]]);
        let last = i16::from_le_bytes([wav[wav.len() - 2], wav[wav.len() - 1]]);
        assert!(first.abs() < 300 && last.abs() < 300, "{first} {last}");
    }

    /// Suena de verdad: `cargo test -p zen-desktop -- --ignored --nocapture suenan`.
    #[test]
    #[ignore = "reproduce sonido"]
    fn suenan_los_cuatro_sonidos() {
        for kind in [SoundKind::Success, SoundKind::Notify, SoundKind::Error, SoundKind::Reminder] {
            play(kind, 15);
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }
}
