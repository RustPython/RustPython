#! /usr/bin/env python
"""Generate Rust code from an ASDL description."""

import datetime
import sys
import json
import textwrap

from argparse import ArgumentParser
from pathlib import Path

import asdl

TABSIZE = 4
AUTOGEN_MESSAGE = "// File automatically generated by {}.\n"

builtin_type_mapping = {
    'identifier': 'Ident',
    'string': 'String',
    'int': 'usize',
    'constant': 'Constant',
    'bool': 'bool',
    'conversion_flag': 'ConversionFlag',
}
assert builtin_type_mapping.keys() == asdl.builtin_types

def get_rust_type(name):
    """Return a string for the C name of the type.

    This function special cases the default types provided by asdl.
    """
    if name in asdl.builtin_types:
        return builtin_type_mapping[name]
    else:
        return "".join(part.capitalize() for part in name.split("_"))

def is_simple(sum):
    """Return True if a sum is a simple.

    A sum is simple if its types have no fields, e.g.
    unaryop = Invert | Not | UAdd | USub
    """
    for t in sum.types:
        if t.fields:
            return False
    return True

def asdl_of(name, obj):
    if isinstance(obj, asdl.Product) or isinstance(obj, asdl.Constructor):
        fields = ", ".join(map(str, obj.fields))
        if fields:
            fields = "({})".format(fields)
        return "{}{}".format(name, fields)
    else:
        if is_simple(obj):
            types = " | ".join(type.name for type in obj.types)
        else:
            sep = "\n{}| ".format(" " * (len(name) + 1))
            types = sep.join(
                asdl_of(type.name, type) for type in obj.types
            )
        return "{} = {}".format(name, types)

class EmitVisitor(asdl.VisitorBase):
    """Visit that emits lines"""

    def __init__(self, file):
        self.file = file
        self.identifiers = set()
        super(EmitVisitor, self).__init__()

    def emit_identifier(self, name):
        name = str(name)
        if name in self.identifiers:
            return
        self.emit("_Py_IDENTIFIER(%s);" % name, 0)
        self.identifiers.add(name)

    def emit(self, line, depth):
        if line:
            line = (" " * TABSIZE * depth) + line
        self.file.write(line + "\n")

class TypeInfo:
    def __init__(self, name):
        self.name = name
        self.has_userdata = None
        self.children = set()
        self.boxed = False

    def __repr__(self):
        return f"<TypeInfo: {self.name}>"

    def determine_userdata(self, typeinfo, stack):
        if self.name in stack:
            return None
        stack.add(self.name)
        for child, child_seq in self.children:
            if child in asdl.builtin_types:
                continue
            childinfo = typeinfo[child]
            child_has_userdata = childinfo.determine_userdata(typeinfo, stack)
            if self.has_userdata is None and child_has_userdata is True:
                self.has_userdata = True

        stack.remove(self.name)
        return self.has_userdata

class FindUserdataTypesVisitor(asdl.VisitorBase):
    def __init__(self, typeinfo):
        self.typeinfo = typeinfo
        super().__init__()

    def visitModule(self, mod):
        for dfn in mod.dfns:
            self.visit(dfn)
        stack = set()
        for info in self.typeinfo.values():
            info.determine_userdata(self.typeinfo, stack)

    def visitType(self, type):
        self.typeinfo[type.name] = TypeInfo(type.name)
        self.visit(type.value, type.name)

    def visitSum(self, sum, name):
        info = self.typeinfo[name]
        if is_simple(sum):
            info.has_userdata = False
        else:
            if len(sum.types) > 1:
                info.boxed = True
            if sum.attributes:
                # attributes means Located, which has the `custom: U` field
                info.has_userdata = True
        for variant in sum.types:
            self.add_children(name, variant.fields)

    def visitProduct(self, product, name):
        info = self.typeinfo[name]
        if product.attributes:
            # attributes means Located, which has the `custom: U` field
            info.has_userdata = True
        if len(product.fields) > 2:
            info.boxed = True
        self.add_children(name, product.fields)

    def add_children(self, name, fields):
        self.typeinfo[name].children.update((field.type, field.seq) for field in fields)

def rust_field(field_name):
    if field_name == 'type':
        return 'type_'
    else:
        return field_name

class TypeInfoEmitVisitor(EmitVisitor):
    def __init__(self, file, typeinfo):
        self.typeinfo = typeinfo
        super().__init__(file)

    def has_userdata(self, typ):
        return self.typeinfo[typ].has_userdata

    def get_generics(self, typ, *generics):
        if self.has_userdata(typ):
            return [f"<{g}>" for g in generics]
        else:
            return ["" for g in generics]

class StructVisitor(TypeInfoEmitVisitor):
    """Visitor to generate typedefs for AST."""

    def visitModule(self, mod):
        for dfn in mod.dfns:
            self.visit(dfn)

    def visitType(self, type, depth=0):
        self.visit(type.value, type.name, depth)

    def visitSum(self, sum, name, depth):
        if is_simple(sum):
            self.simple_sum(sum, name, depth)
        else:
            self.sum_with_constructors(sum, name, depth)

    def emit_attrs(self, depth):
        self.emit("#[derive(Debug, PartialEq)]", depth)

    def simple_sum(self, sum, name, depth):
        rustname = get_rust_type(name)
        self.emit_attrs(depth)
        self.emit(f"pub enum {rustname} {{", depth)
        for variant in sum.types:
            self.emit(f"{variant.name},", depth + 1)
        self.emit("}", depth)
        self.emit("", depth)

    def sum_with_constructors(self, sum, name, depth):
        typeinfo = self.typeinfo[name]
        generics, generics_applied = self.get_generics(name, "U = ()", "U")
        enumname = rustname = get_rust_type(name)
        # all the attributes right now are for location, so if it has attrs we
        # can just wrap it in Located<>
        if sum.attributes:
            enumname = rustname + "Kind"
        self.emit_attrs(depth)
        self.emit(f"pub enum {enumname}{generics} {{", depth)
        for t in sum.types:
            self.visit(t, typeinfo, depth + 1)
        self.emit("}", depth)
        if sum.attributes:
            self.emit(f"pub type {rustname}<U = ()> = Located<{enumname}{generics_applied}, U>;", depth)
        self.emit("", depth)

    def visitConstructor(self, cons, parent, depth):
        if cons.fields:
            self.emit(f"{cons.name} {{", depth)
            for f in cons.fields:
                self.visit(f, parent, "", depth + 1)
            self.emit("},", depth)
        else:
            self.emit(f"{cons.name},", depth)

    def visitField(self, field, parent, vis, depth):
        typ = get_rust_type(field.type)
        fieldtype = self.typeinfo.get(field.type)
        if fieldtype and fieldtype.has_userdata:
            typ = f"{typ}<U>"
        # don't box if we're doing Vec<T>, but do box if we're doing Vec<Option<Box<T>>>
        if fieldtype and fieldtype.boxed and (not field.seq or field.opt):
            typ = f"Box<{typ}>"
        if field.opt:
            typ = f"Option<{typ}>"
        if field.seq:
            typ = f"Vec<{typ}>"
        name = rust_field(field.name)
        self.emit(f"{vis}{name}: {typ},", depth)

    def visitProduct(self, product, name, depth):
        typeinfo = self.typeinfo[name]
        generics, generics_applied = self.get_generics(name, "U = ()", "U")
        dataname = rustname = get_rust_type(name)
        if product.attributes:
            dataname = rustname + "Data"
        self.emit_attrs(depth)
        self.emit(f"pub struct {dataname}{generics} {{", depth)
        for f in product.fields:
            self.visit(f, typeinfo, "pub ", depth + 1)
        self.emit("}", depth)
        if product.attributes:
            # attributes should just be location info
            self.emit(f"pub type {rustname}<U = ()> = Located<{dataname}{generics_applied}, U>;", depth);
        self.emit("", depth)


class FoldTraitDefVisitor(TypeInfoEmitVisitor):
    def visitModule(self, mod, depth):
        self.emit("pub trait Fold<U> {", depth)
        self.emit("type TargetU;", depth + 1)
        self.emit("type Error;", depth + 1)
        self.emit("fn map_user(&mut self, user: U) -> Result<Self::TargetU, Self::Error>;", depth + 2)
        for dfn in mod.dfns:
            self.visit(dfn, depth + 2)
        self.emit("}", depth)

    def visitType(self, type, depth):
        name = type.name
        apply_u, apply_target_u = self.get_generics(name, "U", "Self::TargetU")
        enumname = get_rust_type(name)
        self.emit(f"fn fold_{name}(&mut self, node: {enumname}{apply_u}) -> Result<{enumname}{apply_target_u}, Self::Error> {{", depth)
        self.emit(f"fold_{name}(self, node)", depth + 1)
        self.emit("}", depth)


class FoldImplVisitor(TypeInfoEmitVisitor):
    def visitModule(self, mod, depth):
        self.emit("fn fold_located<U, F: Fold<U> + ?Sized, T, MT>(folder: &mut F, node: Located<T, U>, f: impl FnOnce(&mut F, T) -> Result<MT, F::Error>) -> Result<Located<MT, F::TargetU>, F::Error> {", depth)
        self.emit("Ok(Located { custom: folder.map_user(node.custom)?, location: node.location, node: f(folder, node.node)? })", depth + 1)
        self.emit("}", depth)
        for dfn in mod.dfns:
            self.visit(dfn, depth)

    def visitType(self, type, depth=0):
        self.visit(type.value, type.name, depth)

    def visitSum(self, sum, name, depth):
        apply_t, apply_u, apply_target_u = self.get_generics(name, "T", "U", "F::TargetU")
        enumname = get_rust_type(name)
        is_located = bool(sum.attributes)

        self.emit(f"impl<T, U> Foldable<T, U> for {enumname}{apply_t} {{", depth)
        self.emit(f"type Mapped = {enumname}{apply_u};", depth + 1)
        self.emit("fn fold<F: Fold<T, TargetU = U> + ?Sized>(self, folder: &mut F) -> Result<Self::Mapped, F::Error> {", depth + 1)
        self.emit(f"folder.fold_{name}(self)", depth + 2)
        self.emit("}", depth + 1)
        self.emit("}", depth)

        self.emit(f"pub fn fold_{name}<U, F: Fold<U> + ?Sized>(#[allow(unused)] folder: &mut F, node: {enumname}{apply_u}) -> Result<{enumname}{apply_target_u}, F::Error> {{", depth)
        if is_located:
            self.emit("fold_located(folder, node, |folder, node| {", depth)
            enumname += "Kind"
        self.emit("match node {", depth + 1)
        for cons in sum.types:
            fields_pattern = self.make_pattern(cons.fields)
            self.emit(f"{enumname}::{cons.name} {{ {fields_pattern} }} => {{", depth + 2)
            self.gen_construction(f"{enumname}::{cons.name}", cons.fields, depth + 3)
            self.emit("}", depth + 2)
        self.emit("}", depth + 1)
        if is_located:
            self.emit("})", depth)
        self.emit("}", depth)


    def visitProduct(self, product, name, depth):
        apply_t, apply_u, apply_target_u = self.get_generics(name, "T", "U", "F::TargetU")
        structname = get_rust_type(name)
        is_located = bool(product.attributes)

        self.emit(f"impl<T, U> Foldable<T, U> for {structname}{apply_t} {{", depth)
        self.emit(f"type Mapped = {structname}{apply_u};", depth + 1)
        self.emit("fn fold<F: Fold<T, TargetU = U> + ?Sized>(self, folder: &mut F) -> Result<Self::Mapped, F::Error> {", depth + 1)
        self.emit(f"folder.fold_{name}(self)", depth + 2)
        self.emit("}", depth + 1)
        self.emit("}", depth)

        self.emit(f"pub fn fold_{name}<U, F: Fold<U> + ?Sized>(#[allow(unused)] folder: &mut F, node: {structname}{apply_u}) -> Result<{structname}{apply_target_u}, F::Error> {{", depth)
        if is_located:
            self.emit("fold_located(folder, node, |folder, node| {", depth)
            structname += "Data"
        fields_pattern = self.make_pattern(product.fields)
        self.emit(f"let {structname} {{ {fields_pattern} }} = node;", depth + 1)
        self.gen_construction(structname, product.fields, depth + 1)
        if is_located:
            self.emit("})", depth)
        self.emit("}", depth)

    def make_pattern(self, fields):
        return ",".join(rust_field(f.name) for f in fields)

    def gen_construction(self, cons_path, fields, depth):
        self.emit(f"Ok({cons_path} {{", depth)
        for field in fields:
            name = rust_field(field.name)
            self.emit(f"{name}: Foldable::fold({name}, folder)?,", depth + 1)
        self.emit("})", depth)


class FoldModuleVisitor(TypeInfoEmitVisitor):
    def visitModule(self, mod):
        depth = 0
        self.emit('#[cfg(feature = "fold")]', depth)
        self.emit("pub mod fold {", depth)
        self.emit("use super::*;", depth + 1)
        self.emit("use crate::fold_helpers::Foldable;", depth + 1)
        FoldTraitDefVisitor(self.file, self.typeinfo).visit(mod, depth + 1)
        FoldImplVisitor(self.file, self.typeinfo).visit(mod, depth + 1)
        self.emit("}", depth)


class ClassDefVisitor(EmitVisitor):

    def visitModule(self, mod):
        for dfn in mod.dfns:
            self.visit(dfn)

    def visitType(self, type, depth=0):
        self.visit(type.value, type.name, depth)

    def visitSum(self, sum, name, depth):
        structname = "NodeKind" + get_rust_type(name)
        self.emit(f'#[pyclass(module = "_ast", name = {json.dumps(name)}, base = "AstNode")]', depth)
        self.emit(f'struct {structname};', depth)
        self.emit( '#[pyimpl(flags(HAS_DICT, BASETYPE))]', depth)
        self.emit(f'impl {structname} {{}}', depth)
        for cons in sum.types:
            self.visit(cons, sum.attributes, structname, depth)

    def visitConstructor(self, cons, attrs, base, depth):
        self.gen_classdef(cons.name, cons.fields, attrs, depth, base)

    def visitProduct(self, product, name, depth):
        self.gen_classdef(name, product.fields, product.attributes, depth)

    def gen_classdef(self, name, fields, attrs, depth, base="AstNode"):
        structname = "Node" + name
        self.emit(f'#[pyclass(module = "_ast", name = {json.dumps(name)}, base = {json.dumps(base)})]', depth)
        self.emit(f"struct {structname};", depth)
        self.emit("#[pyimpl(flags(HAS_DICT, BASETYPE))]", depth)
        self.emit(f"impl {structname} {{", depth)
        self.emit(f"#[extend_class]", depth + 1)
        self.emit("fn extend_class_with_fields(ctx: &Context, class: &PyTypeRef) {", depth + 1)
        fields = ",".join(f"ctx.new_str(ascii!({json.dumps(f.name)})).into()" for f in fields)
        self.emit(f'class.set_str_attr("_fields", ctx.new_list(vec![{fields}]));', depth + 2)
        attrs = ",".join(f"ctx.new_str(ascii!({json.dumps(attr.name)})).into()" for attr in attrs)
        self.emit(f'class.set_str_attr("_attributes", ctx.new_list(vec![{attrs}]));', depth + 2)
        self.emit("}", depth + 1)
        self.emit("}", depth)

class ExtendModuleVisitor(EmitVisitor):

    def visitModule(self, mod):
        depth = 0
        self.emit("pub fn extend_module_nodes(vm: &VirtualMachine, module: &PyObject) {", depth)
        self.emit("extend_module!(vm, module, {", depth + 1)
        for dfn in mod.dfns:
            self.visit(dfn, depth + 2)
        self.emit("})", depth + 1)
        self.emit("}", depth)

    def visitType(self, type, depth):
        self.visit(type.value, type.name, depth)

    def visitSum(self, sum, name, depth):
        self.emit(f"{json.dumps(name)} => NodeKind{get_rust_type(name)}::make_class(&vm.ctx),", depth)
        for cons in sum.types:
            self.visit(cons, depth)

    def visitConstructor(self, cons, depth):
        self.gen_extension(cons.name, depth)

    def visitProduct(self, product, name, depth):
        self.gen_extension(name, depth)

    def gen_extension(self, name, depth):
        self.emit(f"{json.dumps(name)} => Node{name}::make_class(&vm.ctx),", depth)


class TraitImplVisitor(EmitVisitor):

    def visitModule(self, mod):
        for dfn in mod.dfns:
            self.visit(dfn)

    def visitType(self, type, depth=0):
        self.visit(type.value, type.name, depth)

    def visitSum(self, sum, name, depth):
        enumname = get_rust_type(name)
        if sum.attributes:
            enumname += "Kind"


        self.emit(f"impl NamedNode for ast::{enumname} {{", depth)
        self.emit(f"const NAME: &'static str = {json.dumps(name)};", depth + 1)
        self.emit("}", depth)
        self.emit(f"impl Node for ast::{enumname} {{", depth)
        self.emit("fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {", depth + 1)
        self.emit("match self {", depth + 2)
        for variant in sum.types:
            self.constructor_to_object(variant, enumname, depth + 3)
        self.emit("}", depth + 2)
        self.emit("}", depth + 1)
        self.emit("fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {", depth + 1)
        self.gen_sum_fromobj(sum, name, enumname, depth + 2)
        self.emit("}", depth + 1)
        self.emit("}", depth)

    def constructor_to_object(self, cons, enumname, depth):
        fields_pattern = self.make_pattern(cons.fields)
        self.emit(f"ast::{enumname}::{cons.name} {{ {fields_pattern} }} => {{", depth)
        self.make_node(cons.name, cons.fields, depth + 1)
        self.emit("}", depth)

    def visitProduct(self, product, name, depth):
        structname = get_rust_type(name)
        if product.attributes:
            structname += "Data"

        self.emit(f"impl NamedNode for ast::{structname} {{", depth)
        self.emit(f"const NAME: &'static str = {json.dumps(name)};", depth + 1)
        self.emit("}", depth)
        self.emit(f"impl Node for ast::{structname} {{", depth)
        self.emit("fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {", depth + 1)
        fields_pattern = self.make_pattern(product.fields)
        self.emit(f"let ast::{structname} {{ {fields_pattern} }} = self;", depth + 2)
        self.make_node(name, product.fields, depth + 2)
        self.emit("}", depth + 1)
        self.emit("fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {", depth + 1)
        self.gen_product_fromobj(product, name, structname, depth + 2)
        self.emit("}", depth + 1)
        self.emit("}", depth)

    def make_node(self, variant, fields, depth):
        lines = []
        self.emit(f"let _node = AstNode.into_ref_with_type(_vm, Node{variant}::static_type().clone()).unwrap();", depth)
        if fields:
            self.emit("let _dict = _node.as_object().dict().unwrap();", depth)
        for f in fields:
            self.emit(f"_dict.set_item({json.dumps(f.name)}, {rust_field(f.name)}.ast_to_object(_vm), _vm).unwrap();", depth)
        self.emit("_node.into()", depth)

    def make_pattern(self, fields):
        return ",".join(rust_field(f.name) for f in fields)

    def gen_sum_fromobj(self, sum, sumname, enumname, depth):
        if sum.attributes:
            self.extract_location(sumname, depth)

        self.emit("let _cls = _object.class();", depth)
        self.emit("Ok(", depth)
        for cons in sum.types:
            self.emit(f"if _cls.is(Node{cons.name}::static_type()) {{", depth)
            self.gen_construction(f"{enumname}::{cons.name}", cons, sumname, depth + 1)
            self.emit("} else", depth)

        self.emit("{", depth)
        msg = f'format!("expected some sort of {sumname}, but got {{}}",_object.repr(_vm)?)'
        self.emit(f"return Err(_vm.new_type_error({msg}));", depth + 1)
        self.emit("})", depth)

    def gen_product_fromobj(self, product, prodname, structname, depth):
        if product.attributes:
            self.extract_location(prodname, depth)

        self.emit("Ok(", depth)
        self.gen_construction(structname, product, prodname, depth + 1)
        self.emit(")", depth)

    def gen_construction(self, cons_path, cons, name, depth):
        self.emit(f"ast::{cons_path} {{", depth)
        for field in cons.fields:
            self.emit(f"{rust_field(field.name)}: {self.decode_field(field, name)},", depth + 1)
        self.emit("}", depth)

    def extract_location(self, typename, depth):
        row = self.decode_field(asdl.Field('int', 'lineno'), typename)
        column = self.decode_field(asdl.Field('int', 'col_offset'), typename)
        self.emit(f"let _location = ast::Location::new({row}, {column});", depth)

    def wrap_located_node(self, depth):
        self.emit(f"let node = ast::Located::new(_location, node);", depth)

    def decode_field(self, field, typename):
        name = json.dumps(field.name)
        if field.opt and not field.seq:
            return f"get_node_field_opt(_vm, &_object, {name})?.map(|obj| Node::ast_from_object(_vm, obj)).transpose()?"
        else:
            return f"Node::ast_from_object(_vm, get_node_field(_vm, &_object, {name}, {json.dumps(typename)})?)?"

class ChainOfVisitors:
    def __init__(self, *visitors):
        self.visitors = visitors

    def visit(self, object):
        for v in self.visitors:
            v.visit(object)
            v.emit("", 0)


def write_ast_def(mod, typeinfo, f):
    f.write('pub use crate::constant::*;\n')
    f.write('pub use crate::location::Location;\n')
    f.write('\n')
    f.write('type Ident = String;\n')
    f.write('\n')
    StructVisitor(f, typeinfo).emit_attrs(0)
    f.write('pub struct Located<T, U = ()> {\n')
    f.write('    pub location: Location,\n')
    f.write('    pub custom: U,\n')
    f.write('    pub node: T,\n')
    f.write('}\n')
    f.write('\n')
    f.write('impl<T> Located<T> {\n')
    f.write('    pub fn new(location: Location, node: T) -> Self {\n')
    f.write('        Self { location, custom: (), node }\n')
    f.write('    }\n')
    f.write('}\n')
    f.write('\n')

    c = ChainOfVisitors(StructVisitor(f, typeinfo),
                        FoldModuleVisitor(f, typeinfo))
    c.visit(mod)


def write_ast_mod(mod, f):
    f.write(textwrap.dedent("""
        #![allow(clippy::all)]

        use super::*;
        use crate::common::ascii;

    """))

    c = ChainOfVisitors(ClassDefVisitor(f),
                        TraitImplVisitor(f),
                        ExtendModuleVisitor(f))
    c.visit(mod)

def main(input_filename, ast_mod_filename, ast_def_filename, dump_module=False):
    auto_gen_msg = AUTOGEN_MESSAGE.format("/".join(Path(__file__).parts[-2:]), datetime.datetime.now(datetime.timezone.utc))
    mod = asdl.parse(input_filename)
    if dump_module:
        print('Parsed Module:')
        print(mod)
    if not asdl.check(mod):
        sys.exit(1)

    typeinfo = {}
    FindUserdataTypesVisitor(typeinfo).visit(mod)

    with ast_def_filename.open("w") as def_file, \
         ast_mod_filename.open("w") as mod_file:
        def_file.write(auto_gen_msg)
        write_ast_def(mod, typeinfo, def_file)

        mod_file.write(auto_gen_msg)
        write_ast_mod(mod, mod_file)

    print(f"{ast_def_filename}, {ast_mod_filename} regenerated.")

if __name__ == "__main__":
    parser = ArgumentParser()
    parser.add_argument("input_file", type=Path)
    parser.add_argument("-M", "--mod-file", type=Path, required=True)
    parser.add_argument("-D", "--def-file", type=Path, required=True)
    parser.add_argument("-d", "--dump-module", action="store_true")

    args = parser.parse_args()
    main(args.input_file, args.mod_file, args.def_file, args.dump_module)
