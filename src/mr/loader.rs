// Copyright 2016 Google Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use binary;
use mr;
use spirv;
use grammar;

use binary::ParseAction;
use std::{error, fmt, result};

#[derive(Debug)]
pub enum Error {
    NestedFunction,
    UnclosedFunction,
    MismatchedFunctionEnd,
    DetachedFunctionParameter,
    DetachedBasicBlock,
    NestedBasicBlock,
    UnclosedBasicBlock,
    MismatchedTerminator,
    DetachedInstruction,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::NestedFunction => write!(f, "found nested function"),
            Error::UnclosedFunction => write!(f, "found unclosed function"),
            Error::MismatchedFunctionEnd => {
                write!(f, "found mismatched OpFunctionEnd")
            }
            Error::DetachedFunctionParameter => {
                write!(f,
                       "found function OpFunctionParameter not inside function")
            }
            Error::DetachedBasicBlock => {
                write!(f, "found basic block not inside function")
            }
            Error::NestedBasicBlock => write!(f, "found nested basic block"),
            Error::UnclosedBasicBlock => {
                write!(f, "found basic block without terminator")
            }
            Error::MismatchedTerminator => {
                write!(f, "found mismatched terminator")
            }
            Error::DetachedInstruction => {
                write!(f, "found instruction not inside basic block")
            }
        }
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            Error::NestedFunction => "found nested function",
            Error::UnclosedFunction => "found unclosed function",
            Error::MismatchedFunctionEnd => "found mismatched OpFunctionEnd",
            Error::DetachedFunctionParameter => {
                "found function OpFunctionParameter not inside function"
            }
            Error::DetachedBasicBlock => {
                "found basic block not inside function"
            }
            Error::NestedBasicBlock => "found nested basic block",
            Error::UnclosedBasicBlock => "found basic block without terminator",
            Error::MismatchedTerminator => "found mismatched terminator",
            Error::DetachedInstruction => {
                "found instruction not inside basic block"
            }
        }
    }
}

type Result<T> = result::Result<T, Error>;

pub struct Loader {
    module: mr::Module,
    function: Option<mr::Function>,
    block: Option<mr::BasicBlock>,
}

impl Loader {
    pub fn new() -> Loader {
        Loader {
            module: mr::Module::new(),
            function: None,
            block: None,
        }
    }

    pub fn module(self) -> mr::Module {
        self.module
    }

    fn require_capability(&mut self, capability: mr::Operand) {
        if let mr::Operand::Capability(cap) = capability {
            self.module
                .capabilities
                .push(cap)

        } else {
            // TODO(antiagainst): we should return a suitable error here.
            panic!()
        }

    }

    fn enable_extension(&mut self, extension: mr::Operand) {
        if let mr::Operand::LiteralString(ext) = extension {
            self.module.extensions.push(ext)
        } else {
            panic!()
        }
    }

    fn attach_name(&mut self, id: mr::Operand, name: mr::Operand) {
        if let (mr::Operand::IdRef(id_ref),
                mr::Operand::LiteralString(name_str)) = (id, name) {
            self.module.names.insert(id_ref, name_str);
        } else {
            panic!()
        }
    }
}

impl binary::Consumer for Loader {
    fn initialize(&mut self) -> ParseAction {
        ParseAction::Continue
    }

    fn finalize(&mut self) -> ParseAction {
        if self.block.is_some() {
            return ParseAction::Error(Box::new(Error::UnclosedBasicBlock));
        }
        if self.function.is_some() {
            return ParseAction::Error(Box::new(Error::UnclosedFunction));
        }
        ParseAction::Continue
    }

    fn consume_header(&mut self, header: mr::ModuleHeader) -> ParseAction {
        self.module.header = Some(header);
        ParseAction::Continue
    }

    fn consume_instruction(&mut self, inst: mr::Instruction) -> ParseAction {
        let mut inst = inst;
        let opcode = inst.class.opcode;
        match opcode {
            spirv::Op::Capability => {
                self.require_capability(inst.operands.pop().unwrap())
            }
            spirv::Op::Extension => {
                self.enable_extension(inst.operands.pop().unwrap())
            }
            spirv::Op::ExtInstImport => {
                self.module
                    .ext_inst_imports
                    .push(inst)
            }
            spirv::Op::MemoryModel => {
                let memory = inst.operands.pop().unwrap();
                let address = inst.operands.pop().unwrap();
                if let (mr::Operand::AddressingModel(am),
                        mr::Operand::MemoryModel(mm)) = (address, memory) {
                    self.module.memory_model = Some((am, mm))
                }
            }
            spirv::Op::EntryPoint => self.module.entry_points.push(inst),
            spirv::Op::ExecutionMode => self.module.execution_modes.push(inst),
            spirv::Op::Name => {
                let name = inst.operands.pop().unwrap();
                let id = inst.operands.pop().unwrap();
                self.attach_name(id, name);
            }
            opcode if grammar::reflect::is_nonlocation_debug(opcode) => {
                self.module.debugs.push(inst)
            }
            opcode if grammar::reflect::is_annotation(opcode) => {
                self.module.annotations.push(inst)
            }
            opcode if grammar::reflect::is_type(opcode) ||
                      grammar::reflect::is_constant(opcode) ||
                      grammar::reflect::is_variable(opcode) => {
                self.module.types_global_values.push(inst)
            }
            spirv::Op::Function => {
                if self.function.is_some() {
                    return ParseAction::Error(Box::new(Error::NestedFunction));
                }
                let mut f = mr::Function::new();
                f.def = Some(inst);
                self.function = Some(f)
            }
            spirv::Op::FunctionEnd => {
                if self.function.is_none() {
                    return ParseAction::Error(Box::new(Error::MismatchedFunctionEnd));
                }
                if self.block.is_some() {
                    return ParseAction::Error(Box::new(Error::UnclosedBasicBlock));
                }
                self.function.as_mut().unwrap().end = Some(inst);
                self.module.functions.push(self.function.take().unwrap())
            }
            spirv::Op::FunctionParameter => {
                if self.function.is_none() {
                    return ParseAction::Error(Box::new(Error::DetachedFunctionParameter));
                }
                self.function.as_mut().unwrap().parameters.push(inst);
            }
            spirv::Op::Label => {
                if self.function.is_none() {
                    return ParseAction::Error(Box::new(Error::DetachedBasicBlock));
                }
                if self.block.is_some() {
                    return ParseAction::Error(Box::new(Error::NestedBasicBlock));
                }
                self.block = Some(mr::BasicBlock::new(inst))
            }
            opcode if grammar::reflect::is_terminator(opcode) => {
                // Make sure the block exists here. Once the block exists,
                // we are certain the function exists because the above checks.
                if self.block.is_none() {
                    return ParseAction::Error(Box::new(Error::MismatchedTerminator));
                }
                self.block
                    .as_mut()
                    .unwrap()
                    .instructions
                    .push(inst);
                self.function
                    .as_mut()
                    .unwrap()
                    .basic_blocks
                    .push(self.block.take().unwrap())
            }
            _ => {
                if self.block.is_none() {
                    return ParseAction::Error(Box::new(Error::DetachedInstruction));
                }
                self.block
                    .as_mut()
                    .unwrap()
                    .instructions
                    .push(inst)
            }
        }
        ParseAction::Continue
    }
}

pub fn load(binary: Vec<u8>) -> Option<mr::Module> {
    let mut loader = Loader::new();
    if let Err(err) = binary::parse(binary, &mut loader) {
        println!("{:?}", err)
    }
    Some(loader.module())
}
