use anyhow::{anyhow, bail, Context, Result};

use crate::ast::*;
use crate::atom::Atom;
use crate::sexpr::{Expr, ExprKind};

pub struct EdifParser {}

macro_rules! next_elem {
    ($it:expr) => {
        $it.next()
            .ok_or_else(|| anyhow!("unexpected end of list"))?
    };
}

macro_rules! ensure_exhausted {
    ($it:expr) => {
        if let Some(item) = $it.next() {
            bail!("list not exhausted at {:?}", item.pos)
        }
    };
}

impl EdifParser {
    pub fn new() -> Self {
        EdifParser {}
    }

    pub fn parse(&self, e: &Expr) -> Result<Edif> {
        let mut it = self.expect_list(e)?.iter();

        self.sym_match(next_elem!(it), atom!("edif"))?;
        let maybe_title = next_elem!(it);
        match &maybe_title.kind {
            ExprKind::Symbol(_) => {
                self.sym_match(&self.expect_list(next_elem!(it))?[0], atom!("edifversion"))?;
            }
            ExprKind::List(list) => self.sym_match(&list[0], atom!("edifversion"))?,
            _ => bail!("Expected sym or list"),
        }

        self.sym_match(&self.expect_list(next_elem!(it))?[0], atom!("edifLevel"))?;
        self.sym_match(&self.expect_list(next_elem!(it))?[0], atom!("keywordmap"))?;
        self.sym_match(&self.expect_list(next_elem!(it))?[0], atom!("status"))?;

        let libs: Vec<Library> = it
            .filter_map(|e| {
                let l = match self.expect_list(e) {
                    Ok(l) => l,
                    Err(e) => return Some(Err(e)),
                };

                if self.sym_match(&l[0], atom!("comment")).is_ok()
                    || self.sym_match(&l[0], atom!("design")).is_ok()
                {
                    None
                } else {
                    Some(self.parse_library(e))
                }
            })
            .collect::<Result<_, _>>()?;

        Ok(Edif { libs })
    }

    fn parse_library(&self, e: &Expr) -> Result<Library> {
        let mut it = self.expect_list(e)?.iter();

        self.sym_match(next_elem!(it), atom!("Library"))?;

        let name = self.expect_sym(next_elem!(it))?;

        self.sym_match(&self.expect_list(next_elem!(it))?[0], atom!("edifLevel"))?;
        self.sym_match(&self.expect_list(next_elem!(it))?[0], atom!("technology"))?;

        let cells = it.map(|e| self.parse_cell(e)).collect::<Result<_, _>>()?;

        Ok(Library { name, cells })
    }

    fn parse_cell(&self, e: &Expr) -> Result<Cell> {
        let mut it = self.expect_list(e)?.iter();

        self.sym_match(next_elem!(it), atom!("cell"))?;

        let name = self.expect_sym(next_elem!(it))?;
        self.sym_match(&self.expect_list(next_elem!(it))?[0], atom!("celltype"))?;
        let view = self.parse_view(next_elem!(it))?;

        ensure_exhausted!(it);

        Ok(Cell { name, view })
    }

    fn parse_view(&self, e: &Expr) -> Result<View> {
        let mut it = self.expect_list(e)?.iter();
        self.sym_match(next_elem!(it), atom!("view"))?;
        let name = self.expect_sym(next_elem!(it))?;
        self.sym_match(&self.expect_list(next_elem!(it))?[0], atom!("viewtype"))?;
        let interface = self.parse_interface(next_elem!(it))?;

        let contents = if let Some(cs) = it.next() {
            let mut cs = self.expect_list(cs)?.iter();
            self.sym_match(next_elem!(cs), atom!("contents"))?;
            cs.map(|e| self.parse_content(e))
                .collect::<Result<_, _>>()?
        } else {
            vec![]
        };

        for e in it {
            let list = self.expect_list(e)?;
            self.sym_match(&list[0], atom!("property"))?;
        }
        // ensure_exhausted!(it);

        Ok(View {
            name,
            interface,
            contents,
        })
    }

    fn parse_interface(&self, e: &Expr) -> Result<Interface> {
        let mut it = self.expect_list(e)?.iter();
        self.sym_match(next_elem!(it), atom!("interface"))?;
        let ports = it
            .map(|p| self.parse_port(p))
            .collect::<Result<Vec<Port>>>()?;
        Ok(Interface { ports })
    }

    fn parse_port(&self, e: &Expr) -> Result<Port> {
        let mut it = self.expect_list(e)?.iter();
        self.sym_match(next_elem!(it), atom!("port"))?;
        let name = next_elem!(it);
        let dir = self.parse_direction(next_elem!(it))?;

        if let Ok(name) = self.parse_name(name) {
            return Ok(Port {
                kind: PortKind::Single,
                dir,
                name,
                is_array: false,
            });
        }

        let list = self.expect_list(name)?;

        anyhow::ensure!(
            list.len() == 3,
            "expected a list with 3-elements at {:?}",
            name.pos,
        );

        self.sym_match(&list[0], atom!("array"))?;

        let size = list[2].num().ok_or_else(|| anyhow!("expected a number"))?;

        let name = self
            .parse_name(&list[1])
            .context("parsing name")?;

        Ok(Port {
            kind: PortKind::Array(size),
            dir,
            name,
            is_array: true,
        })
    }

    fn parse_rename(&self, e: &Expr) -> Result<(Atom, String)> {
        let mut it = self.expect_list(e)?.iter();
        self.sym_match(next_elem!(it), atom!("rename"))?;
        Ok((
            self.expect_sym(next_elem!(it))?,
            self.expect_str(next_elem!(it))?,
        ))
    }

    fn parse_name(&self, e: &Expr) -> Result<Name> {
        if let Ok(name) = self.expect_sym(e) {
            Ok(Name {
                name,
                rename_from: None,
            })
        } else if let Ok((to, from)) = self.parse_rename(e) {
            Ok(Name {
                name: to,
                rename_from: Some(from),
            })
        } else {
            bail!("expected a symbol or '(rename ..)' at {:?}", e.pos);
        }
    }

    fn parse_direction(&self, e: &Expr) -> Result<Direction> {
        let mut it = self.expect_list(e)?.iter();
        self.sym_match(next_elem!(it), atom!("direction"))?;
        let dir = self.expect_sym(next_elem!(it))?;
        Ok(if dir == atom!("INPUT") {
            Direction::Input
        } else if dir == atom!("OUTPUT") {
            Direction::Output
        } else if dir == atom!("INOUT") {
            Direction::InOut
        } else {
            bail!("expected one of 'INPUT', 'OUTPUT', or 'INOUT'");
        })
    }

    fn parse_content(&self, e: &Expr) -> Result<Content> {
        let list = self.expect_list(e)?;
        let sym = self.expect_sym(&list[0])?;

        let name = self.parse_name(&list[1])?;

        if sym == atom!("instance") {
            Ok(Content::Instance(Instance { name }))
        } else if sym == atom!("net") {
            let joined = self.expect_list(&list[2])?;
            self.sym_match(&joined[0], atom!("joined"))?;
            let portrefs = joined[1..].iter().map(|e| {
                self.parse_portref(e)
            }).collect::<Result<Vec<_>>>()?;
            Ok(Content::Net(Net { name, portrefs }))
        } else {
            bail!("expected instance or net at {:?}", e.pos);
        }
    }

    pub fn parse_portref(&self, e: &Expr) -> Result<PortRef> {
        let mut it = self.expect_list(e)?.iter();

        self.sym_match(next_elem!(it), atom!("portref"))?;

        let r = next_elem!(it);

        let (port, member) = match &r.kind {
            ExprKind::List(l) => {
                self.sym_match(&l[0], atom!("member"))?;
                (self.expect_sym(&l[1])?, Some(self.expect_num(&l[2])?))
            }
            ExprKind::Symbol(s) => (s.clone(), None),
            _ => bail!("expected a symbol or '(member ...)' at {:?}", e.pos),
        };

        let instance_ref = if let Some(iref) = it.next() {
            let iref = self.expect_list(iref)?;
            anyhow::ensure!(iref.len() == 2, "expected a list with 2-elements");
            self.sym_match(&iref[0], atom!("instanceref"))?;
            Some(self.expect_sym(&iref[1])?)
        } else {
            None
        };

        Ok(PortRef {
            port,
            member,
            instance_ref,
        })
    }

    fn expect_list<'e>(&self, e: &'e Expr) -> Result<&'e [Expr]> {
        e.list()
            .ok_or_else(|| anyhow!("expected list at {:?}", e.pos))
    }

    fn sym_match(&self, e: &Expr, s: Atom) -> Result<()> {
        if self.expect_sym(e)? == s {
            Ok(())
        } else {
            bail!("expected symbol '{}' but found {:?}", s, e)
        }
    }

    fn expect_sym(&self, e: &Expr) -> Result<Atom> {
        e.symbol()
            .ok_or_else(|| anyhow!("expected a symbol at {:?}", e.pos))
    }

    fn expect_str(&self, e: &Expr) -> Result<String> {
        e.str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("expected a string at {:?}", e.pos))
    }

    fn expect_num(&self, e: &Expr) -> Result<i32> {
        e.num()
            .ok_or_else(|| anyhow!("expected a number at {:?}", e.pos))
    }
}
