use derive_builder::Builder;
use derive_new::new;
use regex::bytes::Regex;

#[derive(Debug, Clone, new, Default)]
pub struct Xf {
    pub fill_id: Option<u32>,
    pub border_id: Option<u32>,
    pub apply_border: bool,
    pub apply_fill: bool,
    pub has_fill_or_border: bool,
    pub has_apply_fill_or_border: bool,
    pub has_only_font_style: bool,
    pub has_apply_protection: bool,
}

#[derive(Debug, Clone, new)]
pub struct CellCoordinate {
    pub column: Vec<u8>,
    pub row: Vec<u8>,
}

#[derive(Debug, Clone, Builder)]
#[builder(setter(into))]
pub struct CleanSheetParameters {
    pub xlsx_file_path: String,
    pub zip_info_filename: String,
    pub compress_method: zip::CompressionMethod,
    
    #[builder(default)]
    pub only_font_style_ids_joined: Option<Vec<u8>>,
    
    #[builder(default)]
    pub fill_or_border_or_protection_ids_joined: Option<Vec<u8>>,
    
    #[builder(default)]
    pub whitespace_indices_joined: Option<Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SheetStreamingState {
    SeekWorksheetNode,
    InWorksheetNode,
    InColsNode,
    InSheetDataNode,
    InMergeCellsNode,
    FoundWorksheetEnd,
}