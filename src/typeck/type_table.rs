use std::collections::HashMap;

/// 类型注册表 — 管理用户定义类型的布局信息
#[derive(Debug, Clone)]
pub struct TypeTable {
    /// 类型名 → 类型定义
    types: HashMap<String, TypeDef>,
}

#[derive(Debug, Clone)]
pub enum TypeDef {
    Struct(StructDef),
    Enum(EnumDef),
}

#[derive(Debug, Clone)]
pub struct StructDef {
    /// 字段名 → 在栈上的偏移量（从结构体基址向栈顶方向计数，0=第一个字段）
    pub fields: Vec<FieldDef>,
    /// 结构体总占用的栈槽数
    pub total_slots: u32,
    /// 是否所有字段均为 Copy 类型
    pub all_copy: bool,
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    /// 字段在结构体中的索引（0-based，按声明顺序）
    pub index: usize,
    /// 字段类型名（用于后续类型检查）
    pub type_name: String,
    /// 该字段占用的槽数（简单类型=1，复合类型=展开槽数）
    pub slots: u32,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    /// 变体列表
    pub variants: Vec<VariantDef>,
    /// 枚举总占用的栈槽数 = 1（标签槽）+ max(变体数据槽数)
    pub total_slots: u32,
}

#[derive(Debug, Clone)]
pub struct VariantDef {
    pub name: String,
    /// 变体标签值（按声明顺序从 0 开始）
    pub tag: u32,
    /// 变体的数据槽数（0 表示无数据）
    pub data_slots: u32,
    /// 数据字段的类型名（简单情况仅一个）
    pub data_type: Option<String>,
}

impl TypeTable {
    pub fn new() -> Self {
        Self { types: HashMap::new() }
    }

    /// 注册结构体类型
    pub fn register_struct(
        &mut self,
        name: &str,
        fields: Vec<(String, String)>, // (字段名, 类型名)
    ) {
        let field_defs: Vec<FieldDef> = fields
            .iter()
            .enumerate()
            .map(|(i, (fname, tname))| FieldDef {
                name: fname.clone(),
                index: i,
                type_name: tname.clone(),
                slots: 1, // 简单类型占 1 槽（复合类型在后续展开）
            })
            .collect();

        let total_slots = field_defs.len() as u32;

        self.types.insert(
            name.to_string(),
            TypeDef::Struct(StructDef {
                fields: field_defs,
                total_slots,
                all_copy: true, // Phase 7 完善
            }),
        );
    }

    /// 注册枚举类型
    pub fn register_enum(
        &mut self,
        name: &str,
        variants: Vec<(String, Option<String>)>, // (变体名, 可选数据类型)
    ) {
        let variant_defs: Vec<VariantDef> = variants
            .iter()
            .enumerate()
            .map(|(i, (vname, dtype))| VariantDef {
                name: vname.clone(),
                tag: i as u32,
                data_slots: if dtype.is_some() { 1 } else { 0 },
                data_type: dtype.clone(),
            })
            .collect();

        let max_data = variant_defs.iter().map(|v| v.data_slots).max().unwrap_or(0);
        let total_slots = 1 + max_data; // tag slot + data

        self.types.insert(
            name.to_string(),
            TypeDef::Enum(EnumDef {
                variants: variant_defs,
                total_slots,
            }),
        );
    }

    /// 遍历所有类型定义
    pub fn iter(&self) -> impl Iterator<Item = (&String, &TypeDef)> {
        self.types.iter()
    }

    /// 查找类型定义
    pub fn lookup(&self, name: &str) -> Option<&TypeDef> {
        self.types.get(name)
    }

    /// 查找结构体定义
    pub fn lookup_struct(&self, name: &str) -> Option<&StructDef> {
        match self.types.get(name) {
            Some(TypeDef::Struct(s)) => Some(s),
            _ => None,
        }
    }

    /// 查找枚举定义
    pub fn lookup_enum(&self, name: &str) -> Option<&EnumDef> {
        match self.types.get(name) {
            Some(TypeDef::Enum(e)) => Some(e),
            _ => None,
        }
    }

    /// 获取结构体字段的栈偏移量
    pub fn field_offset(&self, type_name: &str, field_name: &str) -> Option<u32> {
        let s = self.lookup_struct(type_name)?;
        s.fields.iter().find(|f| f.name == field_name).map(|f| f.index as u32)
    }

    /// 获取枚举变体的标签值
    pub fn variant_tag(&self, type_name: &str, variant_name: &str) -> Option<u32> {
        let e = self.lookup_enum(type_name)?;
        e.variants.iter().find(|v| v.name == variant_name).map(|v| v.tag)
    }

    /// 判断类型是否存在
    pub fn contains(&self, name: &str) -> bool {
        self.types.contains_key(name)
    }

    /// 从 AST 声明构建 TypeTable
    pub fn from_ast_decls(decls: &[crate::frontend::ast::Decl]) -> Self {
        let mut table = Self::new();
        for decl in decls {
            match decl {
                crate::frontend::ast::Decl::Struct(s) => {
                    let fields: Vec<(String, String)> = s
                        .fields
                        .iter()
                        .map(|f| {
                            let tname = type_to_string(&f.ty);
                            (f.name.clone(), tname)
                        })
                        .collect();
                    table.register_struct(&s.name, fields);
                }
                crate::frontend::ast::Decl::Enum(e) => {
                    let variants: Vec<(String, Option<String>)> = e
                        .variants
                        .iter()
                        .map(|v| {
                            let dtype = v.data.as_ref().map(type_to_string);
                            (v.name.clone(), dtype)
                        })
                        .collect();
                    table.register_enum(&e.name, variants);
                }
                _ => {}
            }
        }
        table
    }
}

impl Default for TypeTable {
    fn default() -> Self {
        Self::new()
    }
}

/// 将 AST 类型转为字符串表示（用于类型注册）
fn type_to_string(ty: &crate::frontend::ast::Type) -> String {
    match ty {
        crate::frontend::ast::Type::Path(segments, _) => segments.join("::"),
        crate::frontend::ast::Type::Generic(base, args, _) => {
            let args: Vec<String> = args.iter().map(type_to_string).collect();
            format!("{}<{}>", type_to_string(base), args.join(", "))
        }
        crate::frontend::ast::Type::Ref(inner, mutable, _) => {
            let m = if *mutable { "mut " } else { "" };
            format!("&{m}{}", type_to_string(inner))
        }
        crate::frontend::ast::Type::Tuple(types, _) => {
            let inner: Vec<String> = types.iter().map(type_to_string).collect();
            format!("({})", inner.join(", "))
        }
        crate::frontend::ast::Type::Array(elem, _, _) => {
            format!("[{}]", type_to_string(elem))
        }
        _ => "?".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_struct() {
        let mut table = TypeTable::new();
        table.register_struct("Point", vec![
            ("x".into(), "i32".into()),
            ("y".into(), "i32".into()),
        ]);
        let s = table.lookup_struct("Point").unwrap();
        assert_eq!(s.total_slots, 2);
        assert_eq!(table.field_offset("Point", "x"), Some(0));
        assert_eq!(table.field_offset("Point", "y"), Some(1));
        assert_eq!(table.field_offset("Point", "z"), None);
    }

    #[test]
    fn test_register_enum() {
        let mut table = TypeTable::new();
        table.register_enum("Option", vec![
            ("None".into(), None),
            ("Some".into(), Some("i32".into())),
        ]);
        let e = table.lookup_enum("Option").unwrap();
        assert_eq!(e.total_slots, 2); // 1 tag + 1 data
        assert_eq!(e.variants.len(), 2);
        assert_eq!(table.variant_tag("Option", "None"), Some(0));
        assert_eq!(table.variant_tag("Option", "Some"), Some(1));
    }

    #[test]
    fn test_from_ast() {
        use crate::frontend::parse;
        let src = "
            struct Point { x: i32, y: i32 };
            enum Color { Red, Green, Blue, Rgb(u8, u8, u8) };
        ";
        let (ast, _) = parse(src);
        let table = TypeTable::from_ast_decls(&ast.decls);
        assert!(table.contains("Point"));
        assert!(table.contains("Color"));
    }
}
