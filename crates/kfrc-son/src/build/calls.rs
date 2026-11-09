//! Calls retain one immutable Rust value per logical parameter.

use super::*;

impl Builder<'_> {
    pub(super) fn check_abi(&self, abi: &FunctionAbi) -> Result<(), BuildError> {
        if abi
            .args
            .iter()
            .chain(std::iter::once(&abi.ret))
            .any(|mode| matches!(mode, PassMode::Vector { .. } | PassMode::IndirectOnStack))
        {
            return self
                .unsupported("SIMD and IndirectOnStack ABI are outside the supported scope");
        }
        Ok(())
    }

    pub(super) fn build_call(
        &mut self,
        function: Operand,
        indirect_abi: Option<&kfrc_body::FunctionAbi>,
        args: &[Operand],
        destination: Place,
        target: Option<BlockId>,
        block: BlockId,
        mut control: NodeId,
    ) -> Result<(), BuildError> {
        let index = block.index();
        let source = EdgeSource::Block(block);
        let (symbol, abi) = if let Some(abi) = indirect_abi {
            ("<indirect>".to_owned(), abi.clone())
        } else {
            let Operand::Constant(constant) = function else {
                return self.unsupported("direct call through a non-function operand");
            };
            match &self.body.constant(constant).kind {
                ConstantKind::Function(Callee::Function { symbol, abi }) => {
                    (symbol.clone(), abi.clone())
                }
                ConstantKind::Function(Callee::Intrinsic { name }) => {
                    return self.unsupported(format!("call to intrinsic `{name}`"));
                }
                _ => return self.unsupported("call through a non-function constant"),
            }
        };
        if abi
            .args
            .iter()
            .chain(std::iter::once(&abi.ret))
            .any(|mode| {
                !matches!(
                    mode,
                    PassMode::Direct
                        | PassMode::Ignore
                        | PassMode::Pair
                        | PassMode::Vector { bytes: 16 }
                        | PassMode::Indirect
                        | PassMode::Cast(_)
                )
            })
        {
            return self.unsupported(format!(
                "call to `{symbol}` with ABI {abi:?} (this ABI mode is not lowered yet)"
            ));
        }
        let destination_ty = self.place_type(destination)?;
        self.check_abi(&abi)?;
        let function_value = if indirect_abi.is_some() {
            Some(self.operand(&function, index, &mut control)?)
        } else {
            None
        };
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(self.operand(arg, index, &mut control)?);
        }
        let mut inputs = vec![self.read_variable(self.effect, index)];
        inputs.extend(function_value);
        inputs.extend(values);
        let (opcode, callee_index) = if indirect_abi.is_some() {
            let index = self.graph.call_abis.len() as u32;
            self.graph.call_abis.push(abi);
            (Opcode::CallIndirect, index)
        } else {
            (
                Opcode::Call,
                self.graph.add_callee(CallTarget { symbol, abi }),
            )
        };
        let call = self.graph.add_node(
            opcode,
            control,
            &inputs,
            callee_index,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(destination_ty),
            ]),
        );
        let mut after = self.projection(call, 0, ValueType::Control);
        let effect = self.projection(call, 1, ValueType::Effect);
        self.write_variable(self.effect, index, effect);
        // Every normal return produces the declared Rust value, even when
        // its ABI transports no bits. A diverging call has no value to bind.
        if target.is_some() {
            let result = self.projection(call, 2, ValueType::Rust(destination_ty));
            self.write_place(destination, result, index, &mut after)?;
        }
        match target {
            Some(target) => self.add_edge(source, target, after),
            None => {
                self.graph
                    .add_node(Opcode::Unreachable, after, &[], 0, ValueType::Control);
            }
        }
        Ok(())
    }
}
