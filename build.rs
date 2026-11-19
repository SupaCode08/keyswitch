use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn bgra_top_down_to_icon_dib(size: usize, bgra_top_down: &[u8]) -> Vec<u8> {
    let row_mask_bytes = ((size + 31) / 32) * 4;
    let image_size = (size * size * 4 + row_mask_bytes * size) as u32;
    let mut dib = Vec::with_capacity(40 + image_size as usize);

    let width = size as i32;
    let height = (size * 2) as i32;
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&width.to_le_bytes());
    dib.extend_from_slice(&height.to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&image_size.to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());

    let mut and_mask = vec![0u8; row_mask_bytes * size];

    for y_inv in 0..size {
        let src_y = size - 1 - y_inv;
        for x in 0..size {
            let src_idx = (src_y * size + x) * 4;
            let b = bgra_top_down[src_idx];
            let g = bgra_top_down[src_idx + 1];
            let r = bgra_top_down[src_idx + 2];
            let a = bgra_top_down[src_idx + 3];
            dib.extend_from_slice(&[b, g, r, a]);
            if a < 16 {
                let byte_idx = y_inv * row_mask_bytes + (x / 8);
                let bit_idx = 7 - (x % 8);
                and_mask[byte_idx] |= 1 << bit_idx;
            }
        }
    }

    dib.extend_from_slice(&and_mask);
    dib
}

fn write_multi_ico_file(path: &Path, frames: &[(usize, Vec<u8>)]) {
    let count = frames.len() as u16;
    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&count.to_le_bytes());

    let mut offset = 6u32 + (frames.len() as u32) * 16;
    for (sz, dib) in frames {
        let dim = if *sz >= 256 { 0u8 } else { *sz as u8 };
        ico.push(dim);
        ico.push(dim);
        ico.push(0);
        ico.push(0);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += dib.len() as u32;
    }
    for (_, dib) in frames {
        ico.extend_from_slice(dib);
    }
    let _ = fs::write(path, ico);
}

fn write_multi_coff_rsrc_object(path: &Path, frames: &[(usize, Vec<u8>)]) {
    let n = frames.len();
    let mut grp = Vec::with_capacity(6 + n * 14);
    grp.extend_from_slice(&0u16.to_le_bytes());
    grp.extend_from_slice(&1u16.to_le_bytes());
    grp.extend_from_slice(&(n as u16).to_le_bytes());
    for (i, (sz, dib)) in frames.iter().enumerate() {
        let dim = if *sz >= 256 { 0u8 } else { *sz as u8 };
        grp.push(dim);
        grp.push(dim);
        grp.push(0);
        grp.push(0);
        grp.extend_from_slice(&1u16.to_le_bytes());
        grp.extend_from_slice(&32u16.to_le_bytes());
        grp.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        grp.extend_from_slice(&((i + 1) as u16).to_le_bytes());
    }

    let type3_dir_off = 32u32;
    let type14_dir_off = type3_dir_off + 16 + (n as u32) * 8;
    let lang3_base_off = type14_dir_off + 24;
    let lang14_off = lang3_base_off + (n as u32) * 24;
    let data_entries_off = lang14_off + 24;
    let mut payload_off = data_entries_off + ((n + 1) as u32) * 16;
    while payload_off % 8 != 0 {
        payload_off += 1;
    }

    let mut sec = Vec::new();
    let push_dir = |buf: &mut Vec<u8>, num_id_entries: u16| {
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&num_id_entries.to_le_bytes());
    };
    let push_entry = |buf: &mut Vec<u8>, id: u32, offset: u32, is_dir: bool| {
        buf.extend_from_slice(&id.to_le_bytes());
        let off = if is_dir { offset | 0x8000_0000 } else { offset };
        buf.extend_from_slice(&off.to_le_bytes());
    };

    push_dir(&mut sec, 2);
    push_entry(&mut sec, 3, type3_dir_off, true);
    push_entry(&mut sec, 14, type14_dir_off, true);

    push_dir(&mut sec, n as u16);
    for i in 0..n {
        push_entry(&mut sec, (i + 1) as u32, lang3_base_off + (i as u32) * 24, true);
    }

    push_dir(&mut sec, 1);
    push_entry(&mut sec, 1, lang14_off, true);

    for i in 0..n {
        push_dir(&mut sec, 1);
        push_entry(&mut sec, 0x0409, data_entries_off + (i as u32) * 16, false);
    }

    push_dir(&mut sec, 1);
    push_entry(&mut sec, 0x0409, data_entries_off + (n as u32) * 16, false);

    let mut frame_offsets = Vec::with_capacity(n);
    let mut cur_off = payload_off;
    for (_, dib) in frames {
        frame_offsets.push(cur_off);
        cur_off += dib.len() as u32;
        while cur_off % 8 != 0 {
            cur_off += 1;
        }
    }
    let grp_off = cur_off;

    let mut reloc_offsets = Vec::with_capacity(n + 1);
    for (i, (_, dib)) in frames.iter().enumerate() {
        reloc_offsets.push(sec.len() as u32);
        sec.extend_from_slice(&frame_offsets[i].to_le_bytes());
        sec.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        sec.extend_from_slice(&0u32.to_le_bytes());
        sec.extend_from_slice(&0u32.to_le_bytes());
    }
    reloc_offsets.push(sec.len() as u32);
    sec.extend_from_slice(&grp_off.to_le_bytes());
    sec.extend_from_slice(&(grp.len() as u32).to_le_bytes());
    sec.extend_from_slice(&0u32.to_le_bytes());
    sec.extend_from_slice(&0u32.to_le_bytes());

    while sec.len() < (payload_off as usize) {
        sec.push(0);
    }
    for (i, (_, dib)) in frames.iter().enumerate() {
        while sec.len() < (frame_offsets[i] as usize) {
            sec.push(0);
        }
        sec.extend_from_slice(dib);
    }
    while sec.len() < (grp_off as usize) {
        sec.push(0);
    }
    sec.extend_from_slice(&grp);
    while sec.len() % 8 != 0 {
        sec.push(0);
    }

    let raw_data_ptr = 60u32;
    let reloc_ptr = raw_data_ptr + (sec.len() as u32);
    let symtab_ptr = reloc_ptr + (reloc_offsets.len() as u32) * 10;

    let mut coff = Vec::with_capacity((symtab_ptr + 32) as usize);
    coff.extend_from_slice(&0x8664u16.to_le_bytes());
    coff.extend_from_slice(&1u16.to_le_bytes());
    coff.extend_from_slice(&0u32.to_le_bytes());
    coff.extend_from_slice(&symtab_ptr.to_le_bytes());
    coff.extend_from_slice(&1u32.to_le_bytes());
    coff.extend_from_slice(&0u16.to_le_bytes());
    coff.extend_from_slice(&0u16.to_le_bytes());

    coff.extend_from_slice(b".rsrc\0\0\0");
    coff.extend_from_slice(&0u32.to_le_bytes());
    coff.extend_from_slice(&0u32.to_le_bytes());
    coff.extend_from_slice(&(sec.len() as u32).to_le_bytes());
    coff.extend_from_slice(&raw_data_ptr.to_le_bytes());
    coff.extend_from_slice(&reloc_ptr.to_le_bytes());
    coff.extend_from_slice(&0u32.to_le_bytes());
    coff.extend_from_slice(&(reloc_offsets.len() as u16).to_le_bytes());
    coff.extend_from_slice(&0u16.to_le_bytes());
    coff.extend_from_slice(&0x4000_0040u32.to_le_bytes());

    coff.extend_from_slice(&sec);

    for off in reloc_offsets {
        coff.extend_from_slice(&off.to_le_bytes());
        coff.extend_from_slice(&0u32.to_le_bytes());
        coff.extend_from_slice(&0x0003u16.to_le_bytes());
    }

    coff.extend_from_slice(b".rsrc\0\0\0");
    coff.extend_from_slice(&0u32.to_le_bytes());
    coff.extend_from_slice(&1i16.to_le_bytes());
    coff.extend_from_slice(&0u16.to_le_bytes());
    coff.push(3);
    coff.push(0);

    coff.extend_from_slice(&4u32.to_le_bytes());

    let _ = fs::write(path, coff);
}

fn main() {
    println!("cargo:rerun-if-changed=assets/icon_16.bgra");
    println!("cargo:rerun-if-changed=assets/icon_24.bgra");
    println!("cargo:rerun-if-changed=assets/icon_32.bgra");
    println!("cargo:rerun-if-changed=assets/icon_48.bgra");
    println!("cargo:rerun-if-changed=assets/icon_64.bgra");

    if let Ok(out_dir) = env::var("OUT_DIR") {
        let out_path = PathBuf::from(&out_dir);
        let lib_dir = PathBuf::from(
            r"C:\Users\supak\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\winapi-x86_64-pc-windows-gnu-0.4.0\lib",
        );
        if lib_dir.exists() {
            for name in [
                "shlwapi",
                "shell32",
                "gdi32",
                "user32",
                "advapi32",
                "kernel32",
                "psapi",
                "comctl32",
                "msimg32",
            ] {
                let src = lib_dir.join(format!("libwinapi_{name}.a"));
                let dst = out_path.join(format!("lib{name}.a"));
                if src.exists() && !dst.exists() {
                    let _ = fs::copy(&src, &dst);
                }
            }
            println!("cargo:rustc-link-search=native={}", out_path.display());
        }

        let mut frames = Vec::new();
        for sz in [16usize, 24, 32, 48, 64] {
            let p = PathBuf::from(format!("assets/icon_{sz}.bgra"));
            if let Ok(raw) = fs::read(&p) {
                if raw.len() == sz * sz * 4 {
                    frames.push((sz, bgra_top_down_to_icon_dib(sz, &raw)));
                }
            }
        }
        if !frames.is_empty() {
            write_multi_ico_file(&PathBuf::from("keyswitch.ico"), &frames);
            let obj_path = out_path.join("keyswitch_icon.o");
            write_multi_coff_rsrc_object(&obj_path, &frames);
            println!("cargo:rustc-link-arg={}", obj_path.display());
        }
    }
}
