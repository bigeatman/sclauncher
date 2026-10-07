//! Read-only symbolic fallback for builds whose initialized button tables do not
//! match samase_scarf's disk-table signature. No absolute addresses are assumed.
//! The x64 Windows ABI and command layouts are those used by ShieldBattery.
//! samase_scarf/src/commands.rs documents the append buffer + length invariant.
use samase_scarf::Analysis;
use scarf::analysis::{self, Control, FuncAnalysis};
use scarf::{
    BinaryFile, DestOperand, ExecutionStateX86_64, MemAccessSize, Operand, OperandContext,
    OperandType, Operation, VirtualAddress64,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sender<'e> {
    pub address: VirtualAddress64,
    pub length: Operand<'e>,
    pub buffer: Operand<'e>,
}

/// A fallback succeeds only when exactly one candidate in the supported handler
/// scope has both a recognized caller and a verified append body. Operation or
/// call-budget exhaustion is an error; descendants beyond depth four are outside
/// this resolver's supported scope.
pub(crate) fn resolve<'e>(
    binary: &'e BinaryFile<VirtualAddress64>,
    ctx: &'e OperandContext<'e>,
    analysis: &mut Analysis<'e, ExecutionStateX86_64<'e>>,
) -> Result<Sender<'e>, String> {
    let mut roots = Vec::new();
    if let Some(root) = analysis.game_screen_rclick() {
        roots.push(root);
    }
    if let Some(root) = analysis.targeting_lclick() {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    from_roots(binary, ctx, &roots)
}

fn from_roots<'e>(
    binary: &'e BinaryFile<VirtualAddress64>,
    ctx: &'e OperandContext<'e>,
    roots: &[VirtualAddress64],
) -> Result<Sender<'e>, String> {
    if roots.is_empty() {
        return Err("Sender fallback: mouse-command handlers unresolved".into());
    }
    let code = binary
        .sections()
        .find(|s| s.name == *b".text\0\0\0")
        .ok_or("Sender fallback: code section missing")?;
    let range = (
        code.virtual_address.0,
        code.virtual_address
            .0
            .checked_add(code.data.len() as u64)
            .ok_or("Sender fallback: code range overflow")?,
    );
    let mut finder = CallFinder {
        range,
        calls: 0,
        operations: 0,
        exhausted: false,
        path: Vec::new(),
        candidates: BTreeSet::new(),
    };
    for &root in roots {
        if !(range.0..range.1).contains(&root.0) {
            return Err("Sender fallback: handler outside code".into());
        }
        finder.path.push(root.0);
        FuncAnalysis::new(binary, ctx, root).analyze(&mut finder);
        finder.path.pop();
        if finder.exhausted {
            return Err(format!("Sender fallback: command-handler analysis limit reached (operations={}, calls={}, candidates={}, root={:#x})", finder.operations, finder.calls, finder.candidates.len(), root.0));
        }
    }
    let candidate_count = finder.candidates.len();
    let mut results = Vec::new();
    for address in finder.candidates {
        if let Some(found) = verify_append(binary, ctx, VirtualAddress64(address)) {
            results.push(found);
        }
    }
    match results.len() {
        1 => Ok(results[0]),
        0 => Err(format!(
            "Sender fallback: {candidate_count} command caller candidates; no verified append body"
        )),
        n => Err(format!(
            "Sender fallback: ambiguous ({n} verified append bodies)"
        )),
    }
}

// Initialized builds have substantially more input-helper paths than the tiny
// synthetic fixtures. These fixed budgets cover both depth-four handler graphs;
// exhaustion still rejects any partial search, including a found candidate.
const MAX_HANDLER_OPERATIONS: usize = 1_000_000;
const MAX_HANDLER_CALLS: usize = 4096;

struct CallFinder {
    range: (u64, u64),
    calls: usize,
    operations: usize,
    exhausted: bool,
    path: Vec<u64>,
    candidates: BTreeSet<u64>,
}
impl<'e> analysis::Analyzer<'e> for CallFinder {
    type State = analysis::DefaultState;
    type Exec = ExecutionStateX86_64<'e>;
    fn operation(&mut self, ctrl: &mut Control<'e, '_, '_, Self>, op: &Operation<'e>) {
        self.operations += 1;
        if self.operations > MAX_HANDLER_OPERATIONS || self.exhausted {
            self.exhausted = true;
            ctrl.end_analysis();
            return;
        }
        let Operation::Call(target) = *op else {
            return;
        };
        let Some(address) = ctrl.resolve(target).if_constant() else {
            return;
        };
        if !(self.range.0..self.range.1).contains(&address) {
            return;
        }
        let ctx = ctrl.ctx();
        let source = ctrl.resolve(ctx.register(1)); // RCX
        let len = ctrl.resolve(ctx.register(2)).if_constant(); // RDX
        let marker = ctrl.read_memory(&ctx.mem_access8(source, 0)).if_constant();
        if matches!(
            (marker, len),
            (Some(0x60), Some(12)) | (Some(0x61), Some(13))
        ) {
            self.candidates.insert(address);
            if self.candidates.len() > 32 {
                self.exhausted = true;
                ctrl.end_analysis();
            }
            return;
        }
        // Only explore short descendants of independently resolved input handlers.
        // The call-path check prevents recursion; a global budget bounds diamonds.
        if self.path.len() >= 4 || self.path.contains(&address) {
            return;
        }
        self.calls += 1;
        if self.calls > MAX_HANDLER_CALLS {
            self.exhausted = true;
            ctrl.end_analysis();
            return;
        }
        self.path.push(address);
        ctrl.analyze_with_current_state(self, VirtualAddress64(address));
        self.path.pop();
    }
}

fn strip_sign_extend<'e>(value: Operand<'e>) -> Operand<'e> {
    if let OperandType::SignExtend(inner, MemAccessSize::Mem32, MemAccessSize::Mem64) = *value.ty()
    {
        inner
    } else {
        value
    }
}
fn global_address(value: Operand<'_>) -> Option<u64> {
    let memory = value.if_memory()?;
    if memory.size != MemAccessSize::Mem32 {
        return None;
    }
    memory.if_constant_address()
}
fn in_data(binary: &BinaryFile<VirtualAddress64>, address: u64, size: usize) -> bool {
    binary.sections().any(|s| {
        s.name == *b".data\0\0\0"
            && address >= s.virtual_address.0
            && address
                .checked_add(size as u64)
                .is_some_and(|end| end <= s.virtual_address.0.saturating_add(s.data.len() as u64))
    })
}
fn verify_append<'e>(
    binary: &'e BinaryFile<VirtualAddress64>,
    ctx: &'e OperandContext<'e>,
    address: VirtualAddress64,
) -> Option<Sender<'e>> {
    let mut verifier = AppendVerifier {
        candidate: None,
        results: Vec::new(),
        operations: 0,
        exhausted: false,
    };
    FuncAnalysis::new(binary, ctx, address).analyze(&mut verifier);
    if verifier.exhausted || verifier.results.len() != 1 {
        return None;
    }
    let (buffer, length) = verifier.results[0];
    // Capacity enforcement remains the original sender's responsibility. Require
    // enough captured writable data for the conservative caller budget.
    if !in_data(binary, buffer.if_constant()?, 0x200)
        || !in_data(binary, global_address(length)?, 4)
    {
        return None;
    }
    Some(Sender {
        address,
        length,
        buffer,
    })
}
struct AppendVerifier<'e> {
    candidate: Option<(Operand<'e>, Operand<'e>)>,
    results: Vec<(Operand<'e>, Operand<'e>)>,
    operations: usize,
    exhausted: bool,
}
impl<'e> analysis::Analyzer<'e> for AppendVerifier<'e> {
    type State = analysis::DefaultState;
    type Exec = ExecutionStateX86_64<'e>;
    fn branch_start(&mut self, _: &mut Control<'e, '_, '_, Self>) {
        self.candidate = None;
    }
    fn operation(&mut self, ctrl: &mut Control<'e, '_, '_, Self>, op: &Operation<'e>) {
        self.operations += 1;
        if self.operations > 10_000 {
            self.exhausted = true;
            ctrl.end_analysis();
            return;
        }
        let ctx = ctrl.ctx();
        match *op {
            Operation::Call(_) => {
                self.candidate = None;
                // memcpy-like append must receive both original function inputs.
                let source = ctrl.resolve(ctx.register(2));
                let len = ctrl.resolve(ctx.register(8));
                if source != ctx.register(1)
                    || (len != ctx.register(2)
                        && len != ctx.and_const(ctx.register(2), 0xffff_ffff))
                {
                    return;
                }
                let destination = ctrl.resolve(ctx.register(1));
                let Some((left, right)) = destination.if_arithmetic_add() else {
                    return;
                };
                let pair = if left.if_constant().is_some() {
                    (left, strip_sign_extend(right))
                } else if right.if_constant().is_some() {
                    (right, strip_sign_extend(left))
                } else {
                    return;
                };
                if global_address(pair.1).is_some() {
                    self.candidate = Some(pair);
                }
            }
            Operation::Move(DestOperand::Memory(ref memory), value)
                if memory.size == MemAccessSize::Mem32 =>
            {
                let Some((buffer, length)) = self.candidate else {
                    return;
                };
                let destination = ctx.memory(&ctrl.resolve_mem(memory));
                if destination != length {
                    return;
                }
                let value = ctrl.resolve(value);
                let expected = ctx.and_const(ctx.add(length, ctx.register(2)), 0xffff_ffff);
                if ctx.and_const(value, 0xffff_ffff) == expected
                    && !self.results.contains(&(buffer, length))
                {
                    self.results.push((buffer, length));
                }
                self.candidate = None;
            }
            _ => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarf::BinarySection;
    const CODE: u64 = 0x10000;
    const DATA: u64 = 0x20000;
    const LENGTH: u64 = DATA + 0x200;
    fn rel(code: &mut Vec<u8>, instruction: &[u8], target: u64) {
        code.extend(instruction);
        let end = CODE + code.len() as u64 + 4;
        code.extend(((target as i64 - end as i64) as i32).to_le_bytes());
    }
    fn caller(code: &mut Vec<u8>, sender: u64, marker: u8, length: u8) {
        code.extend([
            0x48, 0x83, 0xec, 0x28, 0xc6, 0x44, 0x24, 0x20, marker, 0x48, 0x8d, 0x4c, 0x24, 0x20,
            0xba, length, 0, 0, 0,
        ]);
        rel(code, &[0xe8], sender);
        code.extend([0x48, 0x83, 0xc4, 0x28, 0xc3]);
    }
    fn append_body(code: &mut Vec<u8>, increment: bool, right_source: bool) {
        // push rbx; rbx=len; r8=len; rdx=source; eax=[length]; rcx=buffer; rcx+=rax
        code.extend([
            0x53,
            0x48,
            0x89,
            0xd3,
            0x49,
            0x89,
            0xd0,
            0x48,
            0x89,
            if right_source { 0xca } else { 0xda },
        ]);
        rel(code, &[0x8b, 0x05], LENGTH);
        rel(code, &[0x48, 0x8d, 0x0d], DATA);
        code.extend([0x48, 0x01, 0xc1]);
        rel(code, &[0xe8], CODE + 0x300);
        if increment {
            rel(code, &[0x01, 0x1d], LENGTH);
        }
        code.extend([0x5b, 0xc3]);
    }
    fn binary(code: Vec<u8>) -> BinaryFile<VirtualAddress64> {
        scarf::raw_bin(
            VirtualAddress64(CODE),
            vec![
                BinarySection {
                    name: *b".text\0\0\0",
                    virtual_address: VirtualAddress64(CODE),
                    virtual_size: code.len() as u32,
                    data: code,
                },
                BinarySection {
                    name: *b".data\0\0\0",
                    virtual_address: VirtualAddress64(DATA),
                    virtual_size: 0x400,
                    data: vec![0; 0x400],
                },
            ],
        )
    }
    fn fixture(
        marker: u8,
        length: u8,
        increment: bool,
        right_source: bool,
    ) -> BinaryFile<VirtualAddress64> {
        let mut code = Vec::new();
        caller(&mut code, CODE + 0x100, marker, length);
        code.resize(0x100, 0xcc);
        append_body(&mut code, increment, right_source);
        code.resize(0x301, 0xc3);
        binary(code)
    }
    #[test]
    fn resolves_sender_from_wire_record_and_exact_append_body() {
        for (marker, length) in [(0x60, 12), (0x61, 13)] {
            let image = fixture(marker, length, true, true);
            let ctx = OperandContext::new();
            let result = from_roots(&image, &ctx, &[VirtualAddress64(CODE)]).unwrap();
            assert_eq!(result.address.0, CODE + 0x100);
            assert_eq!(global_address(result.length), Some(LENGTH));
            assert_eq!(result.buffer.if_constant(), Some(DATA));
        }
    }
    #[test]
    fn rejects_wrong_marker_length_copy_source_or_absent_increment() {
        for (marker, length, increment, source) in [
            (0x60, 13, true, true),
            (0x5c, 12, true, true),
            (0x60, 12, false, true),
            (0x60, 12, true, false),
        ] {
            let image = fixture(marker, length, increment, source);
            let ctx = OperandContext::new();
            assert!(from_roots(&image, &ctx, &[VirtualAddress64(CODE)]).is_err());
        }
    }
    #[test]
    fn accepts_u32_append_length_without_guessing_pointer_width() {
        let mut code = Vec::new();
        caller(&mut code, CODE + 0x100, 0x60, 12);
        code.resize(0x100, 0xcc);
        append_body(&mut code, true, true);
        code[0x104] = 0x41;
        code.resize(0x301, 0xc3);
        let image = binary(code);
        let ctx = OperandContext::new();
        assert!(from_roots(&image, &ctx, &[VirtualAddress64(CODE)]).is_ok());
    }
    #[test]
    fn follows_bounded_input_handler_wrappers() {
        let mut code = vec![0x48, 0x83, 0xec, 0x28];
        rel(&mut code, &[0xe8], CODE + 0x40);
        code.extend([0x48, 0x83, 0xc4, 0x28, 0xc3]);
        code.resize(0x40, 0xcc);
        caller(&mut code, CODE + 0x100, 0x60, 12);
        code.resize(0x100, 0xcc);
        append_body(&mut code, true, true);
        code.resize(0x301, 0xc3);
        let image = binary(code);
        let ctx = OperandContext::new();
        assert!(from_roots(&image, &ctx, &[VirtualAddress64(CODE)]).is_ok());
    }
    #[test]
    fn rejects_truncated_call_graph_even_if_other_root_resolves() {
        let mut code = Vec::new();
        caller(&mut code, CODE + 0x100, 0x60, 12);
        code.resize(0x100, 0xcc);
        append_body(&mut code, true, true);
        code.resize(0x400, 0xc3);
        for _ in 0..=MAX_HANDLER_CALLS {
            rel(&mut code, &[0xe8], CODE + 0x300);
        }
        code.push(0xc3);
        let image = binary(code);
        let ctx = OperandContext::new();
        let err = from_roots(
            &image,
            &ctx,
            &[VirtualAddress64(CODE), VirtualAddress64(CODE + 0x400)],
        )
        .unwrap_err();
        assert!(err.contains("analysis limit"));
    }
    #[test]
    fn refuses_missing_or_out_of_range_handler() {
        let image = fixture(0x60, 12, true, true);
        let ctx = OperandContext::new();
        assert!(from_roots(&image, &ctx, &[]).is_err());
        assert!(from_roots(&image, &ctx, &[VirtualAddress64(DATA)]).is_err());
    }
    #[test]
    fn refuses_multiple_verified_senders() {
        let mut code = Vec::new();
        caller(&mut code, CODE + 0x100, 0x60, 12);
        code.resize(0x40, 0xcc);
        caller(&mut code, CODE + 0x180, 0x61, 13);
        code.resize(0x100, 0xcc);
        append_body(&mut code, true, true);
        code.resize(0x180, 0xcc);
        append_body(&mut code, true, true);
        code.resize(0x301, 0xc3);
        let image = binary(code);
        let ctx = OperandContext::new();
        let error = from_roots(
            &image,
            &ctx,
            &[VirtualAddress64(CODE), VirtualAddress64(CODE + 0x40)],
        )
        .unwrap_err();
        assert!(error.contains("ambiguous"));
    }
}

/// Resolve the dynamic capacity from the sender's pre-call overflow comparison.
/// An append proof alone does not imply a fixed buffer capacity or no flush.
pub(crate) fn resolve_capacity<'e>(
    binary: &'e BinaryFile<VirtualAddress64>,
    ctx: &'e OperandContext<'e>,
    sender: Sender<'e>,
) -> Result<Operand<'e>, String> {
    let mut verifier = CapacityVerifier { length: sender.length, found: Vec::new(), steps: 0 };
    FuncAnalysis::new(binary, ctx, sender.address).analyze(&mut verifier);
    if verifier.steps > 256 || verifier.found.len() != 1 {
        return Err("Sender dynamic capacity comparison unresolved or ambiguous".into());
    }
    let capacity = verifier.found[0];
    if !in_data(binary, global_address(capacity).ok_or("Capacity is not a global u32")?, 4) {
        return Err("Sender capacity is outside game data".into());
    }
    Ok(capacity)
}
struct CapacityVerifier<'e> { length: Operand<'e>, found: Vec<Operand<'e>>, steps: usize }
impl<'e> analysis::Analyzer<'e> for CapacityVerifier<'e> {
    type State = analysis::DefaultState;
    type Exec = ExecutionStateX86_64<'e>;
    fn operation(&mut self, ctrl: &mut Control<'e, '_, '_, Self>, op: &Operation<'e>) {
        self.steps += 1;
        if self.steps > 256 { ctrl.end_analysis(); return; }
        match *op {
            // Only the overflow guard before any helper call can establish this
            // contract. Later obfuscated/network code is not explored here.
            Operation::Call(_) => ctrl.end_analysis(),
            Operation::Jump { condition, .. } => {
                let ctx = ctrl.ctx();
                let incoming = ctx.and_const(ctx.register(2), 0xffff_ffff);
                let total = ctx.and_const(ctx.add(self.length, incoming), 0xffff_ffff);
                find_capacity(ctrl.resolve(condition), total, ctx, 0, &mut self.found);
                if !self.found.is_empty() { ctrl.end_analysis(); }
            }
            _ => (),
        }
    }
}
fn find_capacity<'e>(
    condition: Operand<'e>, total: Operand<'e>, ctx: &'e OperandContext<'e>,
    depth: u8, found: &mut Vec<Operand<'e>>,
) {
    if depth > 8 { return; }
    let OperandType::Arithmetic(ref arith) = *condition.ty() else { return; };
    if arith.ty == scarf::ArithOpType::GreaterThan {
        for (sum, capacity) in [(arith.left, arith.right), (arith.right, arith.left)] {
            if ctx.and_const(sum, 0xffff_ffff) == total && global_address(capacity).is_some()
                && !found.contains(&capacity) { found.push(capacity); }
        }
    }
    find_capacity(arith.left, total, ctx, depth + 1, found);
    find_capacity(arith.right, total, ctx, depth + 1, found);
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    use scarf::BinarySection;
    const CODE: u64 = 0x10000;
    const DATA: u64 = 0x20000;
    const LENGTH: u64 = DATA + 0x200;
    const CAPACITY: u64 = LENGTH + 4;
    fn rel(bytes: &mut Vec<u8>, opcode: &[u8], target: u64) {
        bytes.extend(opcode);
        let end = CODE + bytes.len() as u64 + 4;
        bytes.extend(((target as i64 - end as i64) as i32).to_le_bytes());
    }
    fn image(has_guard: bool, include_incoming: bool) -> BinaryFile<VirtualAddress64> {
        let mut code = Vec::new();
        if has_guard {
            rel(&mut code, &[0x8b, 0x05], LENGTH);
            if include_incoming { code.extend([0x03, 0xc2]); }
            rel(&mut code, &[0x3b, 0x05], CAPACITY);
            code.extend([0x76, 0x01, 0xc3]);
        }
        code.push(0xc3);
        scarf::raw_bin(VirtualAddress64(CODE), vec![
            BinarySection { name: *b".text\0\0\0", virtual_address: VirtualAddress64(CODE), virtual_size: code.len() as u32, data: code },
            BinarySection { name: *b".data\0\0\0", virtual_address: VirtualAddress64(DATA), virtual_size: 0x400, data: vec![0; 0x400] },
        ])
    }
    #[test]
    fn resolves_capacity_only_from_length_plus_incoming_comparison() {
        let ctx = OperandContext::new();
        let binary = image(true, true);
        let sender = Sender { address: VirtualAddress64(CODE), length: ctx.mem32(ctx.constant(LENGTH), 0), buffer: ctx.constant(DATA) };
        let cap = resolve_capacity(&binary, &ctx, sender).unwrap();
        assert_eq!(global_address(cap), Some(CAPACITY));
    }
    #[test]
    fn resolves_r8d_guard_with_preserved_real_sender_input_registers() {
        let mut code = vec![0x48,0x89,0x74,0x24,0x20,0x57,0x48,0x83,0xec,0x20];
        rel(&mut code, &[0x44,0x8b,0x05], LENGTH);
        code.extend([0x48,0x8b,0xf1,0x44,0x03,0xc2,0x8b,0xfa]);
        rel(&mut code, &[0x44,0x3b,0x05], CAPACITY);
        code.extend([0x76,0x01,0xc3,0xc3]);
        let binary = scarf::raw_bin(VirtualAddress64(CODE), vec![
            BinarySection { name: *b".text\0\0\0", virtual_address: VirtualAddress64(CODE), virtual_size: code.len() as u32, data: code },
            BinarySection { name: *b".data\0\0\0", virtual_address: VirtualAddress64(DATA), virtual_size: 0x400, data: vec![0;0x400] },
        ]);
        let ctx = OperandContext::new();
        let sender = Sender { address: VirtualAddress64(CODE), length: ctx.mem32(ctx.constant(LENGTH),0), buffer: ctx.constant(DATA) };
        assert_eq!(global_address(resolve_capacity(&binary,&ctx,sender).unwrap()), Some(CAPACITY));
    }
    #[test]
    fn rejects_absent_guard_or_guard_without_incoming_length() {
        for (guard, incoming) in [(false, false), (true, false)] {
            let ctx = OperandContext::new();
            let binary = image(guard, incoming);
            let sender = Sender { address: VirtualAddress64(CODE), length: ctx.mem32(ctx.constant(LENGTH), 0), buffer: ctx.constant(DATA) };
            assert!(resolve_capacity(&binary, &ctx, sender).is_err());
        }
    }
}