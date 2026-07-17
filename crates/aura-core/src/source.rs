//! Владелец исходников (SPEC §1.2): все `&'a str` лексера/AST/Value заимствуют отсюда.
//! Append-only арена: записи никогда не удаляются и не мутируются.

use std::cell::RefCell;

use crate::span::SourceId;

#[derive(Default)]
pub struct SourceCache {
    files: RefCell<Vec<(String, Box<str>)>>,
}

impl SourceCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Регистрирует исходник и возвращает срез со временем жизни кэша.
    pub fn add(&self, name: String, text: String) -> (SourceId, &str) {
        let boxed = text.into_boxed_str();
        let ptr: *const str = &*boxed;
        let mut files = self.files.borrow_mut();
        files.push((name, boxed));
        let id = (files.len() - 1) as SourceId;
        // SAFETY: данные Box<str> живут в куче по стабильному адресу; Vec двигает только
        // сам Box (указатель), записи из append-only арены никогда не удаляются, поэтому
        // срез валиден, пока жив &self.
        (id, unsafe { &*ptr })
    }

    pub fn name(&self, id: SourceId) -> Option<String> {
        self.files.borrow().get(id as usize).map(|(n, _)| n.clone())
    }

    pub fn text(&self, id: SourceId) -> Option<&str> {
        let files = self.files.borrow();
        let (_, boxed) = files.get(id as usize)?;
        let ptr: *const str = &**boxed;
        // SAFETY: см. `add`.
        Some(unsafe { &*ptr })
    }
}
