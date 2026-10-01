//! Experimental, opt-in preprocessing. Only columns to the right of all cell
//! values/formulas are candidates. Existing cells and non-font style properties
//! are untouched. Missing cells in automatic-height rows veto a font change
//! unless a row font already overrides it. Fixed-height empty positions do not
//! require materialized exceptions. Run normal shrink afterwards.
use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::{collections::{BTreeMap, HashMap}, fs::File, io::{Read,Write}, path::Path, time::Instant};
use zip::{ZipArchive,ZipWriter,write::FileOptions};
fn attr(s:&str,k:&str)->Option<String>{
    Regex::new(&format!(r#"\b{}="([^"]*)""#,regex::escape(k))).unwrap().captures(s).map(|c|c[1].to_owned())
}
fn num(s:&str,k:&str)->u32{attr(s,k).and_then(|x|x.parse().ok()).unwrap_or(0)}
fn set(s:&str,k:&str,v:u32)->String{
    let re=Regex::new(&format!(r#"\b{}="[^"]*""#,regex::escape(k))).unwrap();
    let a=format!("{k}=\"{v}\"");
    if re.is_match(s){re.replace(s,a.as_str()).into_owned()}else{
        let i=s.find('>').unwrap();let i=if s.as_bytes()[i-1]==b'/'{i-1}else{i};
        format!("{} {}{}",&s[..i],a,&s[i..])
    }
}
fn coord(s:&str)->(usize,u32){let mut c=0;let mut r=0;for b in s.bytes(){if b.is_ascii_uppercase(){c=c*26+(b-b'A'+1) as usize}else if b.is_ascii_digit(){r=r*10+(b-b'0') as u32;}}(c,r)}
#[derive(Clone)]struct Style{xml:String,font:u32}
struct Row{number:u32,fixed:bool,font:Option<u32>,cells:HashMap<usize,u32>}
fn normalize(xml:&str, styles:&mut Vec<Style>)->Result<(String,usize)>{
    // Avoid unknown protection/conditional formatting interactions in this trial.
    if xml.contains("<sheetProtection") || xml.contains("<conditionalFormatting"){return Ok((xml.into(),0));}
    let cols_re=Regex::new(r"(?s)<cols\b[^>]*>.*?</cols>")?;
    let Some(cols)=cols_re.find(xml) else{return Ok((xml.into(),0))};
    let col_re=Regex::new(r"<col\s[^>]*/>")?;
    let row_re=Regex::new(r"(?s)<row\b[^>]*>.*?</row>")?;
    let cell_re=Regex::new(r"(?s)<c\b[^>]*(?:/>|>.*?</c>)")?;
    let content_re=Regex::new(r"<(?:v|is|f)(?:\s|>|/)")?;
    let mut rows=Vec::new(); let mut max_col=0;let mut max_row=0;
    for m in row_re.find_iter(xml){
        let raw=m.as_str();let head=&raw[..=raw.find('>').unwrap()];
        let row_style=attr(head,"s").and_then(|x|x.parse::<usize>().ok()).and_then(|i|styles.get(i));
        let font=if attr(head,"customFormat").as_deref()==Some("1"){row_style.map(|s|s.font)}else{None};
        let mut row=Row{number:num(head,"r"),fixed:attr(head,"customHeight").as_deref()==Some("1"),font,cells:HashMap::new()};
        for c in cell_re.find_iter(raw){
            let h=&c.as_str()[..=c.as_str().find('>').unwrap()];
            let (col,r)=coord(&attr(h,"r").unwrap_or_default());
            if col==0||col>16384{continue;}
            if content_re.is_match(c.as_str()){max_col=max_col.max(col);max_row=max_row.max(r);}
            let sid=num(h,"s") as usize;
            // Explicit styles are retained verbatim, including values and exceptions.
            if attr(h,"s").is_some(){if let Some(s)=styles.get(sid){row.cells.insert(col,s.font);}}
        }
        rows.push(row);
    }
    if max_row==0{return Ok((xml.into(),0));}
    rows.retain(|r|r.number<=max_row);
    // Missing worksheet rows are automatic-height rows. Conservatively skip.
    if rows.len()!=max_row as usize{return Ok((xml.into(),0));}
    let mut counts:HashMap<usize,BTreeMap<u32,usize>>=HashMap::new();
    for row in &rows{for (&col,&font) in &row.cells{if col>max_col{*counts.entry(col).or_default().entry(font).or_default()+=1;}}}
    let mut changed=0;let mut new_cols=cols.as_str().to_owned();let mut edits=Vec::new();
    for m in col_re.find_iter(cols.as_str()){
        let tag=m.as_str();let lo=num(tag,"min") as usize;let hi=num(tag,"max") as usize;
        if lo==0||hi>16384||lo>hi{continue;}
        let old_id=num(tag,"style") as usize;
        let Some(old)=styles.get(old_id).cloned() else{continue};
        let mut runs:Vec<(usize,usize,usize)>=Vec::new();
        for col in lo..=hi{
            let candidate=counts.get(&col).and_then(|c|c.iter().max_by_key(|(font,n)|(**n,std::cmp::Reverse(**font))).map(|(&f,_)|f));
            let mut sid=old_id;
            if let Some(font)=candidate.filter(|&f|f!=old.font){
                let safe=rows.iter().all(|r|r.fixed||r.cells.contains_key(&col)||r.font==Some(font));
                if safe{
                    // Clone raw XF; changing font cannot discard number format,
                    // alignment, protection, fill, border or future attributes.
                    let xf=set(&set(&old.xml,"fontId",font),"applyFont",1);
                    sid=styles.iter().position(|s|s.xml==xf).unwrap_or_else(||{let i=styles.len();styles.push(Style{xml:xf,font});i});
                    changed+=1;
                }
            }
            if let Some(last)=runs.last_mut(){if last.2==sid{last.1=col;continue;}}
            runs.push((col,col,sid));
        }
        if runs.len()!=1||runs[0].2!=old_id{
            let replacement=runs.into_iter().map(|(a,b,id)|set(&set(&set(tag,"min",a as u32),"max",b as u32),"style",id as u32)).collect::<String>();
            edits.push((m.start(),m.end(),replacement));
        }
    }
    for (a,b,s) in edits.into_iter().rev(){new_cols.replace_range(a..b,&s);}
    let mut out=xml.to_owned();out.replace_range(cols.range(),&new_cols);Ok((out,changed))
}
fn main()->Result<()>{
 let args:Vec<_>=std::env::args().collect();ensure!(args.len()>=3,"usage: font_inversion_probe INPUT OUTPUT [sheet10.xml sheet11.xml ...]");
 let input=Path::new(&args[1]);let output=Path::new(&args[2]);ensure!(!output.exists(),"output already exists");
 let started=Instant::now();let mut zip=ZipArchive::new(File::open(input)?)?;
 let mut styles_xml=String::new();zip.by_name("xl/styles.xml")?.read_to_string(&mut styles_xml)?;
 let xfs_re=Regex::new(r"(?s)<cellXfs\b[^>]*>.*?</cellXfs>")?;
 let block=xfs_re.find(&styles_xml).context("cellXfs missing")?;
 let xf_re=Regex::new(r"(?s)<xf\b[^>]*(?:/>|>.*?</xf>)")?;
 let mut styles:Vec<_>=xf_re.find_iter(block.as_str()).map(|m|Style{xml:m.as_str().into(),font:num(m.as_str().split('>').next().unwrap(),"fontId")}).collect();
 let initial=styles.len();let mut writer=ZipWriter::new(File::create(output)?);let opts=FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
 for i in 0..zip.len(){
  let mut entry=zip.by_index(i)?;let name=entry.name().to_owned();
  if name=="xl/styles.xml"{continue;}
  let selected=name.starts_with("xl/worksheets/sheet")&&name.ends_with(".xml")&&(args.len()==3||args[3..].iter().any(|s|name.ends_with(&format!("/{s}"))));
  if selected{
   let mut xml=String::new();entry.read_to_string(&mut xml)?;let (out,count)=normalize(&xml,&mut styles)?;
   println!("{name}: changed_columns={count}");writer.start_file(name,opts)?;writer.write_all(out.as_bytes())?;
  }else{writer.raw_copy_file(entry)?;}
 }
 let open_end=block.as_str().find('>').unwrap()+1;
 let head=set(&block.as_str()[..open_end],"count",styles.len() as u32);
 let new_block=format!("{}{}{}",head,styles.iter().map(|s|s.xml.as_str()).collect::<String>(),"</cellXfs>");
 let range=block.range();styles_xml.replace_range(range,&new_block);
 writer.start_file("xl/styles.xml",opts)?;writer.write_all(styles_xml.as_bytes())?;writer.finish()?;
 println!("added_styles={} elapsed_ms={}",styles.len()-initial,started.elapsed().as_millis());Ok(())
}
#[cfg(test)]mod tests{
 use super::*;
 fn styles()->Vec<Style>{vec![Style{xml:r#"<xf fontId="23" numFmtId="49" fillId="0" borderId="0"/>"#.into(),font:23},Style{xml:r#"<xf fontId="18"/>"#.into(),font:18}]}
 fn sheet(fixed:&str)->String{format!(r#"<worksheet><cols><col min="1" max="3" style="0" width="12"/></cols><sheetData><row r="1" {fixed}><c r="A1"><v>1</v></c><c r="B1" s="1"/><c r="C1" s="1"/></row><row r="2" {fixed}><c r="A2"><v>2</v></c></row></sheetData></worksheet>"#)}
 #[test]fn fixed_rows_allow_inversion_without_changing_cells_or_other_style_fields(){let mut s=styles();let x=sheet(r#"customHeight="1" ht="20""#);let (out,n)=normalize(&x,&mut s).unwrap();assert_eq!(n,2);assert!(out.contains(r#"min="2" max="3" style="2""#));assert!(s[2].xml.contains(r#"numFmtId="49""#));assert_eq!(s[2].font,18);assert_eq!(out.split("<sheetData>").nth(1),x.split("<sheetData>").nth(1));}
 #[test]fn implicit_auto_height_cells_veto_change(){let (_,n)=normalize(&sheet(""),&mut styles()).unwrap();assert_eq!(n,0);}
 #[test]fn missing_rows_veto_change(){let x=sheet("customHeight=\"1\"").replace("r=\"2\"","r=\"3\"").replace("A2","A3");let (_,n)=normalize(&x,&mut styles()).unwrap();assert_eq!(n,0);}
}
