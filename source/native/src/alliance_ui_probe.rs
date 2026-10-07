//! Bounded, read-only UI metadata for identifying the current alliance dialog.
//! No executable bytes, addresses, game simulation data, or chat history are exported.
//! SCR layout: pinned bw_dat structs.rs/dialog.rs (see FEASIBILITY.md).
use std::collections::HashSet;

const MAX_ROOTS: usize = 128;
const MAX_CHILDREN: usize = 256;
const HEADER: usize = 0x78;
const FIRST_CHILD: usize = 0x90;

fn word(bytes: &[u8], at: usize) -> usize {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
}
fn pointer(p: usize, n: usize) -> bool {
    p >= 0x10000 && p % 8 == 0 && p.checked_add(n).is_some()
}
fn number(bytes: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(128).collect()
}
fn name<F>(p: usize, bytes: &[u8], read: &mut F) -> Option<String>
where F: FnMut(usize, &mut [u8]) -> bool {
    let length = word(bytes, 0x28);
    if length == 0 { return Some(String::new()); }
    if length > 192 { return None; }
    let data = word(bytes, 0x20);
    let raw_cap = word(bytes, 0x30);
    let inline = raw_cap & (1usize << 63) != 0;
    let cap = raw_cap & !(1usize << 63);
    if cap < length || cap > 8192 || data < 0x10000 || data.checked_add(length + 1).is_none()
        || (inline && (data != p.checked_add(0x38)? || length >= 16)) { return None; }
    let mut value = vec![0; length + 1];
    if !read(data, &mut value) { return None; }
    if value[length] != 0 { return None; }
    let text = std::str::from_utf8(&value[..length]).ok()?;
    Some(text.to_owned())
}
fn record(kind: &str, bytes: &[u8], text: &str) -> String {
    let flags = u32::from_le_bytes(bytes[0x48..0x4c].try_into().unwrap());
    let flags2 = u32::from_le_bytes(bytes[0x4c..0x50].try_into().unwrap());
    format!("{kind}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{text}\n",
        number(bytes,0x52), number(bytes,0x54), u8::from(flags & 2 != 0),
        u8::from(flags2 & 1 == 0), number(bytes,8),number(bytes,10),
        number(bytes,12),number(bytes,14),text=clean(text))
}

pub struct Metadata { pub key: String, pub text: String, pub candidate: bool }

/// Root-list names are inventoried. Children are read ONLY for the documented
/// classic alliance resource candidate. Its name is not treated as proof that
/// a working SCR feature can be enabled; the metadata is for subsequent review.
pub fn inspect<F>(first: usize, mut read: F) -> Option<Metadata>
where F: FnMut(usize, &mut [u8]) -> bool {
    let mut seen = HashSet::new();
    let mut current = first;
    let mut text = String::new();
    let mut candidate = false;
    while current != 0 {
        if seen.len() >= MAX_ROOTS || !pointer(current, HEADER) || !seen.insert(current) { return None; }
        let mut bytes = [0; HEADER];
        if !read(current, &mut bytes) { return None; }
        let label = name(current, &bytes, &mut read)?;
        if number(&bytes,0x54) != 0 { return None; }
        let next = word(&bytes,0);
        let line = record("ROOT",&bytes,&label);
        text.push_str(&line);
        let visible = u32::from_le_bytes(bytes[0x48..0x4c].try_into().unwrap()) & 2 != 0;
        if label == "AllyFltr" && !visible { text.push_str("HIDDEN_CANDIDATE\n"); }
        if (label == "AllyFltr" || label == "Minimap") && visible {
            candidate |= label == "AllyFltr";
            let mut child_pointer = [0;8];
            if !read(current.checked_add(FIRST_CHILD)?, &mut child_pointer) { return None; }
            let mut child = word(&child_pointer,0);
            let mut children = HashSet::new();
            while child != 0 {
                if children.len() >= MAX_CHILDREN || !pointer(child,HEADER)
                    || seen.contains(&child) || !children.insert(child) { return None; }
                let mut child_bytes = [0;HEADER];
                if !read(child,&mut child_bytes) { return None; }
                if word(&child_bytes,0x70) != current { return None; }
                // Minimap diagnostics contain only numeric ID/type/visibility/
                // bounds for the two known controls; no child strings are read.
                let child_label = if label == "AllyFltr" {Some(name(child,&child_bytes,&mut read)?)}else{None};
                if let Some(child_label)=child_label.as_ref() {
                    text.push_str(&record("CHILD",&child_bytes,child_label));
                } else if matches!(number(&child_bytes,0x52),3|4) {
                    text.push_str(&record("MINIMAP_BUTTON",&child_bytes,""));
                }
                let mut again = [0;HEADER];
                if !read(child,&mut again) || again != child_bytes
                    || child_label.as_ref().is_some_and(|expected|name(child,&again,&mut read).as_ref()!=Some(expected)) { return None; }
                child = word(&child_bytes,0);
            }
            let mut check_child = [0;8];
            if !read(current.checked_add(FIRST_CHILD)?,&mut check_child)
                || check_child != child_pointer { return None; }
        }
        let mut again = [0;HEADER];
        if !read(current,&mut again) || again != bytes || name(current,&again,&mut read)? != label { return None; }
        current = next;
    }
    let key = text.clone();
    Some(Metadata {key,text,candidate})
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn minimap_diagnostics_only_record_pair_metadata_without_child_labels() {
        let mut f=Fixture::new();f.control(BASE,0,"Minimap");f.set(BASE,FIRST_CHILD,BASE+0x400);
        f.control(BASE+0x400,BASE+0x600,"private child label");
        f.control(BASE+0x600,BASE+0x800,"other private label");f.control(BASE+0x800,0,"unrelated label");
        for (address,id) in [(BASE+0x400,3u16),(BASE+0x600,4),(BASE+0x800,5)] {
            f.set(address,0x70,BASE);
            let start=address-BASE+0x52;f.bytes[start..start+2].copy_from_slice(&id.to_le_bytes());
            f.set(address,0x20,1); // Not a valid string pointer; must not be read.
        }
        let metadata=inspect(BASE,|at,out|f.read(at,out)).unwrap();
        assert_eq!(metadata.text.matches("MINIMAP_BUTTON").count(),2);
        assert!(!metadata.text.contains("private"));assert!(!metadata.candidate);
        f.set(BASE+0x600,0x70,BASE+0x200);
        assert!(inspect(BASE,|at,out|f.read(at,out)).is_none());
    }
    const BASE:usize=0x10000;
    struct Fixture { bytes:Vec<u8> }
    impl Fixture {
        fn new()->Self { Self {bytes:vec![0;0x2000]} }
        fn set(&mut self, at:usize, field:usize, value:usize) {
            let start=at-BASE+field;
            self.bytes[start..start+8].copy_from_slice(&(value as u64).to_le_bytes());
        }
        fn control(&mut self,at:usize,next:usize,label:&str) {
            self.set(at,0,next); self.set(at,0x20,at+0xa0);
            self.set(at,0x28,label.len()); self.set(at,0x30,0x100);
            let start=at-BASE+0xa0; self.bytes[start..start+label.len()].copy_from_slice(label.as_bytes());
            self.bytes[at-BASE+0x48]=2;
        }
        fn read(&self,at:usize,out:&mut[u8])->bool {
            let Some(start)=at.checked_sub(BASE) else{return false;};
            let Some(end)=start.checked_add(out.len()) else{return false;};
            let Some(src)=self.bytes.get(start..end) else{return false;};
            out.copy_from_slice(src);true
        }
    }
    #[test] fn only_alliance_candidate_children_are_read() {
        let mut f=Fixture::new(); f.control(BASE,BASE+0x200,"ChatLog");
        f.set(BASE,FIRST_CHILD,1); // Invalid, but unrelated dialog children must not be followed.
        f.control(BASE+0x200,0,"AllyFltr"); f.set(BASE+0x200,FIRST_CHILD,BASE+0x400);
        f.control(BASE+0x400,0,"확인");
        f.set(BASE+0x400,0x70,BASE+0x200);
        let m=inspect(BASE,|at,out|f.read(at,out)).unwrap();
        assert!(m.text.contains("ROOT\t")); assert!(m.text.contains("확인"));
        assert_eq!(m.text.matches("CHILD\t").count(),1);
        assert!(m.key.contains("확인"));
    }
    #[test] fn cycles_and_invalid_inline_names_reject_metadata() {
        let mut f=Fixture::new(); f.control(BASE,BASE,"AllyFltr");
        assert!(inspect(BASE,|at,out|f.read(at,out)).is_none());
        f.set(BASE,0,0); f.set(BASE,0x30,(1usize<<63)|15);
        assert!(inspect(BASE,|at,out|f.read(at,out)).is_none());
    }
    #[test] fn changing_header_rejects_metadata() {
        let mut f=Fixture::new();f.control(BASE,0,"StatBtn");let mut reads=0;
        assert!(inspect(BASE,|at,out| { let ok=f.read(at,out); if at==BASE {reads+=1;if reads==2 {out[0x52]=1;} } ok }).is_none());
    }
    #[test] fn correct_inline_strings_are_supported_and_controls_are_removed() {
        let mut f=Fixture::new();f.control(BASE,0,"StatBtn");f.set(BASE,0x20,BASE+0x38);
        f.set(BASE,0x30,(1usize<<63)|15);f.bytes[0x38..0x3f].copy_from_slice(b"StatBtn");
        assert!(inspect(BASE,|at,out|f.read(at,out)).unwrap().text.ends_with("StatBtn\n"));
        assert_eq!(clean("a\tb\nc\0"),"abc");
    }
    #[test] fn child_layout_is_from_pinned_repr_c() {
        #[repr(C)] struct Rect {l:i16,t:i16,r:i16,b:i16}
        #[repr(C)] struct Surface {w:u16,h:u16,data:*mut u8}
        #[repr(C)] struct BwString {data:*const u8,len:usize,cap:usize,inline:[u8;16]}
        #[repr(C)] struct Control {next:*mut Control,area:Rect,image:Surface,string:BwString,
            flags:u32,flags2:u32,unknown:u16,id:i16,ty:u16,misc:u16,user:*mut u8,
            callback:*mut u8,draw:*mut u8,parent:*mut u8}
        #[repr(C)] struct Dialog {control:Control,surface:Surface,highlighted:*mut Control,first_child:*mut Control,active:*mut Control}
        assert_eq!(std::mem::size_of::<Control>(),0x78);
        assert_eq!(std::mem::offset_of!(Dialog,first_child),FIRST_CHILD);
    }
    #[test] fn sanitized_alias_does_not_authorize_child_reads() {
        let mut f=Fixture::new(); f.control(BASE,0,"Ally\0Fltr"); f.set(BASE,FIRST_CHILD,1);
        let m=inspect(BASE,|at,out|f.read(at,out)).unwrap();
        assert!(!m.candidate);assert!(!m.text.contains("CHILD"));
    }
    #[test] fn hidden_candidate_skips_children_and_wrong_parent_rejects() {
        let mut f=Fixture::new(); f.control(BASE,0,"AllyFltr");f.set(BASE,FIRST_CHILD,1);f.bytes[0x48]=0;
        let m=inspect(BASE,|at,out|f.read(at,out)).unwrap();assert!(!m.candidate);assert!(m.text.contains("HIDDEN_CANDIDATE"));
        f.bytes[0x48]=2;f.set(BASE,FIRST_CHILD,BASE+0x200);f.control(BASE+0x200,0,"확인");
        assert!(inspect(BASE,|at,out|f.read(at,out)).is_none());
    }
}
