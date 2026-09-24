/// Tipos de layout de disco Xbox 360, identificados pelo deslocamento em que
/// a assinatura "MICROSOFT*XBOX*MEDIA" é encontrada dentro do setor 32.
///
/// Corresponde a `Chilano.Xbox360.Iso.IsoType` no projeto original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoIso {
    Xsf,
    Xgd1,
    Xgd2,
    Xgd3,
}

impl TipoIso {
    /// Nome curto do layout, para etiquetas de tabela e telas.
    pub fn rotulo(self) -> &'static str {
        match self {
            TipoIso::Xsf => "Xsf",
            TipoIso::Xgd1 => "XGD1",
            TipoIso::Xgd2 => "XGD2",
            TipoIso::Xgd3 => "XGD3",
        }
    }

    /// Deslocamento, em bytes, do início do volume dentro do setor de assinatura.
    pub fn deslocamento_raiz(self) -> u64 {
        match self {
            TipoIso::Xsf => 0,
            TipoIso::Xgd1 => 405_798_912,
            TipoIso::Xgd2 => 265_879_552,
            TipoIso::Xgd3 => 34_078_720,
        }
    }
}

/// Descritor de volume GDF, equivalente a `GDFVolumeDescriptor`.
#[derive(Debug, Clone)]
pub struct DescritorVolume {
    pub identificador: Vec<u8>,
    pub setor_dir_raiz: u32,
    pub tamanho_dir_raiz: u32,
    pub criacao_imagem: Vec<u8>,
    pub tamanho_setor: u32,
    pub deslocamento_raiz: u64,
    pub tamanho_volume: u64,
    pub setores_volume: u32,
}
