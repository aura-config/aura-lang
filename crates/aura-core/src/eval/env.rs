//! Лексическая область видимости (SPEC §4.2, D9).
//!
//! Реализация D9: вместо полной цепочки замороженных префиксов фрейм использует
//! внутреннюю мутабельность (RefCell) на этапе построения блока. Значения при этом
//! иммутабельны, переопределение в одном фрейме запрещено (E0301 в интерпретаторе),
//! ссылки идут строго вверх — циклы Arc невозможны. Наблюдаемое отличие от строгой
//! заморозки: замыкание видит биндинги своего блока, объявленные ПОСЛЕ него; вызвать
//! его раньше их объявления в детерминированном top-down порядке исполнения нельзя.

use indexmap::IndexMap;
use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

use super::value::Value;

pub type Env<'a> = Arc<Environment<'a>>;

pub struct Environment<'a> {
    vars: RefCell<IndexMap<String, Value<'a>>>,
    parent: Option<Env<'a>>,
}

impl<'a> Environment<'a> {
    pub fn root() -> Env<'a> {
        Arc::new(Environment { vars: RefCell::new(IndexMap::new()), parent: None })
    }

    pub fn child(parent: &Env<'a>) -> Env<'a> {
        Arc::new(Environment { vars: RefCell::new(IndexMap::new()), parent: Some(parent.clone()) })
    }

    /// Подъём по цепочке фреймов; клон значения — O(1) (Arc).
    pub fn get(&self, name: &str) -> Option<Value<'a>> {
        if let Some(v) = self.vars.borrow().get(name) {
            return Some(v.clone());
        }
        self.parent.as_ref().and_then(|p| p.get(name))
    }

    pub fn defined_here(&self, name: &str) -> bool {
        self.vars.borrow().contains_key(name)
    }

    pub fn defined_in_ancestors(&self, name: &str) -> bool {
        match &self.parent {
            Some(p) => p.defined_here(name) || p.defined_in_ancestors(name),
            None => false,
        }
    }

    /// Низкоуровневая вставка; правила E0301/E0302 применяет интерпретатор.
    pub fn insert(&self, name: &str, v: Value<'a>) {
        self.vars.borrow_mut().insert(name.to_string(), v);
    }
}

impl fmt::Debug for Environment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<env: {} vars>", self.vars.borrow().len())
    }
}
