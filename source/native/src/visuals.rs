//! Read-only visual metadata for an external click-through overlay.
//! No native sprite flags, images, UI selection, or simulation fields are written.
use crate::memory::{Value, read_integer, read_memory};

pub struct Config {
    pub screen_x: Value,
    pub screen_y: Value,
    pub zoom: Value,
    pub width: Value,
    pub height: Value,
    pub units_dat: Value,
    pub dat_stride: usize,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub x: i32,
    pub y: i32,
    pub zoom: f32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub ring_width: u32,
    pub ring_height: u32,
    pub max_hp: u32,
    pub max_shield: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub appearance: Appearance,
    pub hp: u32,
    pub shield: u32,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub frame: u32,
    pub view: View,
    pub markers: Vec<Marker>,
}
impl Config {
    pub fn view(&self) -> Option<View> {
        let x = self.screen_x.read()? as u32 as i32;
        let y = self.screen_y.read()? as u32 as i32;
        let zoom = f32::from_bits(u32::try_from(self.zoom.read()?).ok()?);
        let width = u32::try_from(self.width.read()?).ok()?;
        let height = u32::try_from(self.height.read()?).ok()?;
        valid_view(View {x,y,zoom,width,height})
    }
    pub fn appearance(&self, kind: u16) -> Option<Appearance> {
        let table = self.units_dat.read()?;
        if self.dat_stride != 16 { return None; }
        let field = |index, size| -> Option<usize> {
            let address = table.checked_add(index * self.dat_stride)?;
            let data = read_integer(address,8)?;
            let item_size = read_integer(address.checked_add(8)?,4)?;
            let entries = read_integer(address.checked_add(12)?,4)?;
            if item_size != size || kind as usize >= entries || entries > 65536 { return None; }
            data.checked_add(kind as usize * size)
        };
        let hp = u32::try_from(read_integer(field(8,4)?,4)?).ok()?;
        if hp == 0 || hp > i32::MAX as u32 { return None; }
        let has_shield = read_integer(field(6,1)?,1)?;
        if has_shield > 1 { return None; }
        let max_shield = if has_shield == 1 { u32::try_from(read_integer(field(7,2)?,2)?).ok()?.checked_mul(256)? } else { 0 };
        // units.dat field 38 stores collision extents; this overlay draws an
        // ellipse around that footprint, rather than modifying native images.
        let mut dimensions = [0;8];
        if !read_memory(field(38,8)?, &mut dimensions) { return None; }
        let left = u16::from_le_bytes(dimensions[0..2].try_into().ok()?) as u32;
        let right = u16::from_le_bytes(dimensions[4..6].try_into().ok()?) as u32;
        if left > 256 || right > 256 { return None; }
        let ring_width = ((left + right + 1) * 13 / 10).clamp(32,192);
        Some(Appearance {ring_width,ring_height:ring_width/2,max_hp:hp,max_shield})
    }
}
pub fn valid_view(view: View) -> Option<View> {
    if !(-32768..=32768).contains(&view.x) || !(-32768..=32768).contains(&view.y)
        || !view.zoom.is_finite() || !(0.125..=16.0).contains(&view.zoom)
        || !(64..=16384).contains(&view.width) || !(64..=16384).contains(&view.height) {
        None
    } else { Some(view) }
}
pub fn read_marker(pointer: usize, id: u32, owner: u8, view: View, appearance: Appearance) -> Option<Marker> {
    if owner >= 8 || id == 0 { return None; }
    let mut unit = [0u8;0x90];
    if !read_memory(pointer, &mut unit) || unit[0x68] != owner { return None; }
    let sprite = u64::from_le_bytes(unit[0x18..0x20].try_into().ok()?) as usize;
    let mut visible = [0u8;3];
    if !read_memory(sprite.checked_add(0x14)?, &mut visible)
        || visible[0] & (1u8 << owner) == 0 || visible[2] & 0x20 != 0 { return None; }
    let hp = i32::from_le_bytes(unit[0x10..0x14].try_into().ok()?);
    let shield = i32::from_le_bytes(unit[0x88..0x8c].try_into().ok()?);
    if hp <= 0 { return None; }
    let x = i16::from_le_bytes(unit[0x40..0x42].try_into().ok()?) as i32 - view.x;
    let y = i16::from_le_bytes(unit[0x42..0x44].try_into().ok()?) as i32 - view.y;
    let radius = appearance.ring_width as i32;
    if x < -radius || y < -radius || x > view.width as i32 + radius || y > view.height as i32 + radius { return None; }
    Some(Marker { id,x,y,appearance,hp:hp as u32,shield:shield.max(0) as u32 })
}
impl Frame {
    pub fn encode(&self, pid: u32) -> String {
        let mut out = format!("SCVIS1\t{pid}\t{}\t{}\t{}\t{}\t{}\n",self.frame,self.view.zoom,self.view.width,self.view.height,self.markers.len());
        for m in &self.markers {
            out.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",m.id,m.x,m.y,m.appearance.ring_width,m.appearance.ring_height,m.hp,m.appearance.max_hp,m.shield,m.appearance.max_shield));
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn view() -> View { View{x:100,y:200,zoom:2.0,width:640,height:383} }
    fn appearance() -> Appearance { Appearance{ring_width:40,ring_height:20,max_hp:5120,max_shield:5120} }
    #[test]
    fn metadata_uses_owned_x64_unit_sprite_fixtures() {
        let mut sprite=[0u8;0x20]; sprite[0x14]=1<<7;
        let mut unit=[0u8;0x90];
        unit[0x18..0x20].copy_from_slice(&(sprite.as_ptr() as u64).to_le_bytes());
        unit[0x10..0x14].copy_from_slice(&3840i32.to_le_bytes());
        unit[0x88..0x8c].copy_from_slice(&1280i32.to_le_bytes());
        unit[0x40..0x42].copy_from_slice(&300i16.to_le_bytes());
        unit[0x42..0x44].copy_from_slice(&500i16.to_le_bytes());
        unit[0x68]=7;
        let pointer=unit.as_ptr() as usize;
        let marker=read_marker(pointer,8193,7,view(),appearance()).unwrap();
        assert_eq!((marker.x,marker.y,marker.hp,marker.shield),(200,300,3840,1280));
        assert!(read_marker(pointer,8193,6,view(),appearance()).is_none());
        sprite[0x14]=0; std::hint::black_box(&sprite); assert!(read_marker(pointer,8193,7,view(),appearance()).is_none());
        sprite[0x14]=1<<7; sprite[0x16]=0x20; std::hint::black_box(&sprite);
        assert!(read_marker(pointer,8193,7,view(),appearance()).is_none());
    }
    #[test]
    fn dat_table_fixture_proves_field_widths_stride_and_fixed_point_health() {
        #[repr(C)] struct DatTable { data:usize, width:u32, entries:u32 }
        assert_eq!(std::mem::size_of::<DatTable>(),16);
        let enabled=[1u8;228]; let shields=[20u16;228]; let hp=[10240i32;228];
        let dimensions=[[15u16,10,16,11];228];
        let mut dat:Vec<DatTable>=(0..39).map(|_|DatTable{data:0,width:0,entries:0}).collect();
        dat[6]=DatTable{data:enabled.as_ptr() as usize,width:1,entries:228};
        dat[7]=DatTable{data:shields.as_ptr() as usize,width:2,entries:228};
        dat[8]=DatTable{data:hp.as_ptr() as usize,width:4,entries:228};
        dat[38]=DatTable{data:dimensions.as_ptr() as usize,width:8,entries:228};
        let config=Config{screen_x:Value::Constant(100),screen_y:Value::Constant(200),zoom:Value::Constant(2.0f32.to_bits() as usize),width:Value::Constant(640),height:Value::Constant(383),units_dat:Value::Constant(dat.as_ptr() as usize),dat_stride:16};
        assert_eq!(config.view(),Some(view()));
        assert_eq!(config.appearance(64),Some(Appearance{ring_width:41,ring_height:20,max_hp:10240,max_shield:5120}));
        dat[8].width=8; std::hint::black_box(&dat);
        assert!(config.appearance(64).is_none());
    }
    #[test]
    fn nonfinite_or_unbounded_projection_is_rejected() {
        assert!(valid_view(view()).is_some());
        for zoom in [f32::NAN,f32::INFINITY,0.0,17.0] { assert!(valid_view(View{zoom,..view()}).is_none()); }
        assert!(valid_view(View{width:0,..view()}).is_none());
    }
    #[test]
    fn wire_contains_current_not_assumed_full_health() {
        let f=Frame{frame:42,view:view(),markers:vec![Marker{id:9,x:10,y:20,appearance:appearance(),hp:2560,shield:0}]};
        let text=f.encode(701);
        assert!(text.starts_with("SCVIS1\t701\t42\t2\t640\t383\t1\n"));
        assert!(text.ends_with("9\t10\t20\t40\t20\t2560\t5120\t0\t5120\n"));
    }
}
