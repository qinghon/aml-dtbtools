use clap::Parser;
use flate2::read::GzDecoder;
use std::cmp::min;
use std::fs::{self, File};
use std::io::prelude::*;
use std::io::{self, SeekFrom, Write};
use std::mem::size_of;
use std::str;
use std::str::FromStr;
use std::{path, vec};

const AML_DT_VERSION: u32 = 2;
const DT_ID_TAG: &str = "amlogic-dt-id";
const PAGE_SIZE_DEF: usize = 2048;
const PAGE_SIZE_MAX: usize = 1024 * 1024;
// const COPY_BLK: usize = 1024;
const INFO_ENTRY_SIZE: usize = 16;

const AML_DT_MAGIC: &[u8; 4] = b"AML_";
const AML_DT_HEADER: u32 = 0x5f4c4d41;
const DT_HEADER_MAGIC: u32 = 0xedfe0dd0;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub(crate) struct SplitArgs {
    #[arg(short, long)]
    boot_img_path: String,

    #[arg(short, long)]
    dest: String,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub(crate) struct PackArgs {
    #[arg(short, long)]
    out_file: String,
    #[arg(short, long, default_value_t = PAGE_SIZE_DEF as u32, value_parser = clap::value_parser!(u32).range(0..(PAGE_SIZE_MAX as i64)))]
    page_size: u32,
    #[arg(short, long)]
    input_dir: String,
}

#[repr(C)]
struct DTHeader {
    magic: u32,
    totalsize: u32,
}

#[repr(C)]
struct Header {
    magic: u32,
    version: u32,
    entry_count: u32,
}

#[repr(C)]
struct HeaderEntry<const ID_SIZE: usize> {
    soc: [u8; ID_SIZE],
    plat: [u8; ID_SIZE],
    vari: [u8; ID_SIZE],
    offset: u32,
    dtb_size: u32,
}

impl<const ID_SIZE: usize> HeaderEntry<ID_SIZE> {
    fn new() -> HeaderEntry<{ ID_SIZE }> {
        Self {
            soc: [0; ID_SIZE],
            plat: [0; ID_SIZE],
            vari: [0; ID_SIZE],
            offset: 0,
            dtb_size: 0,
        }
    }
}

pub trait AsByteSlice {
    fn as_slice(self: &Self) -> &[u8]
    where
        Self: Sized,
    {
        unsafe {
            core::slice::from_raw_parts(
                (self as *const Self) as *const u8,
                ::core::mem::size_of::<Self>(),
            )
        }
    }
    fn as_mut_slice(self: &mut Self) -> &mut [u8]
    where
        Self: Sized,
    {
        unsafe {
            core::slice::from_raw_parts_mut(
                (self as *mut Self) as *mut u8,
                ::core::mem::size_of::<Self>(),
            )
        }
    }
}

type HeaderEntryV1 = HeaderEntry<4>;
type HeaderEntryV2 = HeaderEntry<16>;

impl AsByteSlice for Header {}
impl AsByteSlice for DTHeader {}
impl<const ID_SIZE: usize> AsByteSlice for HeaderEntry<ID_SIZE> {}

trait SeekRead: Seek + Read {}
impl<T: Seek + Read> SeekRead for T {}

fn dump_data<const ID_SIZE: usize>(
    entries: u32,
    dest: &str,
    dtb: &mut dyn SeekRead,
) -> io::Result<()> {
    let mut headers: Vec<HeaderEntry<ID_SIZE>> = Vec::new();

    for _ in 0..entries {
        let mut h = HeaderEntry::<ID_SIZE>::new();
        let h_bytes = h.as_mut_slice();
        dtb.read_exact(h_bytes)?;
        headers.push(h);
    }

    for h in headers.iter_mut() {
        for chunk in h.soc.chunks_mut(4) {
            chunk.reverse();
        }
        for chunk in h.plat.chunks_mut(4) {
            chunk.reverse();
        }
        for chunk in h.vari.chunks_mut(4) {
            chunk.reverse();
        }
        let mut id = String::new();
        id.push_str(str::from_utf8(&h.soc).unwrap().trim_end());
        id.push('-');
        id.push_str(str::from_utf8(&h.plat).unwrap().trim_end());
        id.push('-');
        id.push_str(str::from_utf8(&h.vari).unwrap().trim_end());

        println!("Found header: {}", id);

        dtb.seek(SeekFrom::Start(h.offset as u64))?;
        let mut dtheader = DTHeader {
            magic: 0,
            totalsize: 0,
        };
        let dtheader_bytes = dtheader.as_mut_slice();
        dtb.read_exact(dtheader_bytes)?;
        if dtheader.magic != DT_HEADER_MAGIC {
            println!("\tDTB Header mismatch. Found: {:x}", dtheader.magic);
            continue;
        }

        dtheader.totalsize = u32::from_be(dtheader.totalsize);
        println!("\t offset: {} size: {}", h.offset, dtheader.totalsize);

        dtb.seek(SeekFrom::Start(h.offset as u64))?;
        let mut data = vec![0; dtheader.totalsize as usize];
        dtb.read_exact(&mut data)?;

        let output_path = format!("{}{}.dtb", dest, id);
        let mut output = File::create(output_path)?;
        output.write_all(&data)?;
    }

    Ok(())
}

pub fn dtb_split(split_arg: &SplitArgs) -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} boot.img out_prefix", args[0]);
        return Ok(());
    }

    let boot_img_path = &split_arg.boot_img_path;
    let dest = &split_arg.dest;

    let mut dtb = File::open(boot_img_path)?;
    let mut header = Header {
        magic: 0,
        version: 0,
        entry_count: 0,
    };

    let header_bytes = header.as_mut_slice();

    dtb.read_exact(header_bytes)?;
    let mut dtb_reader;
    
    if header.magic != AML_DT_HEADER {
        if header.magic & 0xffff == 0x8b1f {
            dtb.seek(SeekFrom::Start(0 as u64))?;

            let mut d = GzDecoder::new(dtb);
            let mut data = Vec::new();
            d.read_to_end(&mut data)
                .expect("cannot decompression gzip file");

            dtb_reader = io::Cursor::new(data);
    
            let header_bytes = header.as_mut_slice();
            dtb_reader.read_exact(header_bytes)?;
            if header.magic != AML_DT_HEADER {
                eprintln!("Invalid AML DTB header.");
                return Ok(());
            }
        } else {
            eprintln!("Invalid AML DTB header.");
            return Ok(());
        }
    } else {
        let mut data = Vec::new();
        dtb.read_to_end(&mut data)
            .expect("cannot read dtb file");
        dtb_reader = io::Cursor::new(data);
    };

    println!(
        "DTB Version: {} entries: {}",
        header.version, header.entry_count
    );

    match header.version {
        1 => dump_data::<4>(header.entry_count, &dest, &mut dtb_reader)?,
        2 => dump_data::<16>(header.entry_count, &dest, &mut dtb_reader)?,
        _ => {
            eprintln!("Unrecognized DTB version");
            return Ok(());
        }
    }

    Ok(())
}

#[derive(Debug)]
struct ChipInfo {
    chipset: [u8; INFO_ENTRY_SIZE],
    platform: [u8; INFO_ENTRY_SIZE],
    rev_num: [u8; INFO_ENTRY_SIZE],
    dtb_size: u32,
    dtb_file: Vec<u8>,
}

impl ChipInfo {
    fn new() -> Self {
        Self {
            chipset: [0; INFO_ENTRY_SIZE],
            platform: [0; INFO_ENTRY_SIZE],
            rev_num: [0; INFO_ENTRY_SIZE],
            dtb_size: 0,
            dtb_file: vec![],
        }
    }
}

fn pad_spaces(s: &mut [u8]) {
    let len = s.len();
    for i in (0..len).rev() {
        if s[i] == 0 {
            s[i] = b' ';
        } else {
            break;
        }
    }
}

fn copy_str_to_cstr(dst: &mut [u8; INFO_ENTRY_SIZE], src: &str) {
    for i in 0..min(src.len(), INFO_ENTRY_SIZE - 1) {
        let c = src.chars().nth(i).unwrap();
        if !c.is_ascii() || c.is_whitespace() {
            break;
        }

        dst[i] = c as u8
    }
    dst[INFO_ENTRY_SIZE - 1] = 0;
}

fn get_chip_info(filename: &str, page_size: usize) -> Option<ChipInfo> {
    let mut input = fs::File::open(filename).unwrap();
    let mut buf = Vec::new();
    input.read_to_end(&mut buf).unwrap();

    let dt = device_tree::DeviceTree::load(buf.as_slice()).unwrap();

    if let Some(node) = dt.find("/") {
        match node.prop_str(DT_ID_TAG) {
            Ok(s) => {
                let sp: Vec<&str> = s.split(&['_', '-']).collect();

                if sp.len() != 3 {
                    eprintln!("cannot parse {}: {}", DT_ID_TAG, s);
                }
                let mut chip = ChipInfo::new();
                copy_str_to_cstr(&mut chip.chipset, sp[0]);
                copy_str_to_cstr(&mut chip.platform, sp[1]);
                copy_str_to_cstr(&mut chip.rev_num, sp[2]);
                chip.dtb_size = (buf.len() + (page_size - (buf.len() % page_size))) as u32;
                chip.dtb_file = buf;
                return Some(chip);
            }
            Err(_) => {
                eprintln!("cannot find {} in device tree", DT_ID_TAG);
                return None;
            }
        }
    }

    println!("cannot find {} in {}", DT_ID_TAG, filename);
    None
}

pub fn dtb_pack(args: &PackArgs) {
    println!("DTB combiner:");
    println!("  Input directory: '{}'", args.input_dir);
    println!("  Output file: '{}'", args.out_file);
    let input_dir = &args.input_dir;
    let page_size = args.page_size as usize;
    let out_file = &args.out_file;

    let filler = vec![0u8; page_size];
    let mut chip_list: Vec<ChipInfo> = vec![];

    if let Ok(entries) = fs::read_dir(input_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().unwrap_or_default() == "dtb" {
                println!("Found file: {:?}", path.file_name().unwrap());
                if let Some(chip) = get_chip_info(path.to_str().unwrap(), page_size) {
                    chip_list.push(chip);
                } else {
                    println!("skip, failed to scan for '{}'", DT_ID_TAG);
                }
            }
        }
    }

    let dtb_count = chip_list.len();
    println!("=> Found {} unique DTB(s)", dtb_count);
    if dtb_count == 0 {
        chip_list.clear();
        return;
    }

    let mut fp_out = File::create(path::PathBuf::from_str(&out_file).unwrap())
        .expect("Error opening output file");

    let h = Header {
        magic: AML_DT_HEADER,
        version: AML_DT_VERSION,
        entry_count: dtb_count as u32,
    };
    fp_out
        .write_all(h.as_slice())
        .expect("Error writing header");

    let mut dtb_offset = size_of::<Header>() + size_of::<HeaderEntryV2>() * dtb_count + 4;
    let padding = page_size - (dtb_offset % page_size);
    dtb_offset += padding;
    let mut expected = dtb_offset;

    for chip in chip_list.iter_mut() {
        pad_spaces(&mut chip.chipset);
        pad_spaces(&mut chip.platform);
        pad_spaces(&mut chip.rev_num);

        for chunk in chip.chipset.chunks_mut(4) {
            chunk.reverse();
        }
        for chunk in chip.platform.chunks_mut(4) {
            chunk.reverse();
        }
        for chunk in chip.rev_num.chunks_mut(4) {
            chunk.reverse();
        }
        let entry = HeaderEntryV2 {
            soc: chip.chipset,
            plat: chip.platform,
            vari: chip.rev_num,
            offset: expected as u32,
            dtb_size: chip.dtb_size,
        };
        fp_out
            .write_all(entry.as_slice())
            .expect("failed write entry header");

        expected += chip.dtb_size as usize;
    }

    let rc: u32 = 0;
    fp_out
        .write(&rc.to_le_bytes())
        .expect("cannot wirte status ");

    if padding > 0 {
        fp_out
            .write_all(&filler[0..padding])
            .expect("cannot write filler");
    }

    for chip in chip_list.iter_mut() {
        let dtb_buf = &chip.dtb_file;
        io::copy(&mut io::Cursor::new(dtb_buf), &mut fp_out).expect("Error copying dtb file");

        let filler_size = page_size - (dtb_buf.len() % page_size);
        if filler_size > 0 && filler_size < page_size {
            fp_out
                .write_all(&filler[0..filler_size])
                .expect("Error writing filler");
        }
    }

    fp_out.flush().expect("cannot flush output");
    drop(fp_out);

    println!("Output written to '{}'", out_file);
}
