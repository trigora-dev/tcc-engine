use super::*;

impl Engine {
    pub(super) fn array_index(&mut self, index: u32) -> Result<Option<EngineOutcome>, CoreError> {
        let top = self
            .continuation
            .stack
            .last()
            .cloned()
            .ok_or(CoreError::StackUnderflow)?;
        let id = self.expect_ref(top, "ArrayIndex requires an array")?;
        let value = match self.continuation.heap.get(id as usize) {
            Some(HeapCell::Array(items)) => items
                .get(index as usize)
                .cloned()
                .unwrap_or(Value::Undefined),
            _ => {
                return Err(CoreError::TypeError(
                    "ArrayIndex requires an array".to_string(),
                ))
            }
        };
        self.continuation.stack.push(value);
        self.advance_pc().map(|()| None)
    }

    pub(super) fn alloc_cell(&mut self, cell: HeapCell) -> u32 {
        let id = self.continuation.heap.len() as u32;
        self.continuation.heap.push(cell);
        id
    }

    pub(super) fn expect_ref(&self, value: Value, message: &str) -> Result<u32, CoreError> {
        match value {
            Value::Ref(id) => Ok(id),
            _ => Err(CoreError::TypeError(message.to_string())),
        }
    }

    pub(super) fn guard_structure(&self, id: u32) -> Result<(), CoreError> {
        if self.continuation.iterating.contains(&id) {
            Err(CoreError::TypeError(
                "cannot change a collection while it is being iterated".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    pub(super) fn get_index(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let index_value = self.pop()?;
        let collection = self.pop()?;
        let id = self.expect_ref(collection, "index requires a collection")?;
        let lang = self.continuation.language_semantics_version.clone();
        let index = match crate::compute::index_number(&lang, index_value) {
            Ok(index) => index,
            Err(crate::compute::ArithFail::Type(message)) => {
                return Err(CoreError::TypeError(message.to_string()))
            }
            Err(crate::compute::ArithFail::Raise(message)) => {
                return self.throw_value(Value::String(message.to_string()))
            }
        };
        let len = match self.continuation.heap.get(id as usize) {
            Some(HeapCell::Array(items)) => items.len(),
            _ => {
                return Err(CoreError::TypeError(
                    "index requires a list or array".to_string(),
                ))
            }
        };
        let resolved = if lang == LANGUAGE_SEMANTICS_PY && index < 0 {
            len as i64 + index
        } else {
            index
        };
        if resolved < 0 || resolved as usize >= len {
            if lang == LANGUAGE_SEMANTICS_PY {
                return self.throw_value(Value::String("IndexError".into()));
            }
            self.continuation.stack.push(Value::Undefined);
        } else {
            let value = match self.continuation.heap.get(id as usize) {
                Some(HeapCell::Array(items)) => items[resolved as usize].clone(),
                _ => Value::Undefined,
            };
            self.continuation.stack.push(value);
        }
        self.advance_pc().map(|()| None)
    }

    pub(super) fn set_index(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let value = self.pop()?;
        let index_value = self.pop()?;
        let collection = self.pop()?;
        let id = self.expect_ref(collection, "index assignment requires a collection")?;
        self.guard_structure(id)?;
        let lang = self.continuation.language_semantics_version.clone();
        let index = match crate::compute::index_number(&lang, index_value) {
            Ok(index) => index,
            Err(crate::compute::ArithFail::Type(message)) => {
                return Err(CoreError::TypeError(message.to_string()))
            }
            Err(crate::compute::ArithFail::Raise(message)) => {
                return self.throw_value(Value::String(message.to_string()))
            }
        };
        let len = match self.continuation.heap.get(id as usize) {
            Some(HeapCell::Array(items)) => items.len(),
            _ => {
                return Err(CoreError::TypeError(
                    "index assignment requires a list or array".to_string(),
                ))
            }
        };
        let resolved = if lang == LANGUAGE_SEMANTICS_PY && index < 0 {
            len as i64 + index
        } else {
            index
        };
        if resolved < 0 || resolved as usize >= len {
            return Err(CoreError::TypeError(
                "index assignment is out of range".to_string(),
            ));
        }
        match self.continuation.heap.get_mut(id as usize) {
            Some(HeapCell::Array(items)) => items[resolved as usize] = value,
            _ => {
                return Err(CoreError::TypeError(
                    "index assignment requires a list or array".to_string(),
                ))
            }
        }
        self.continuation.stack.push(Value::Ref(id));
        self.advance_pc().map(|()| None)
    }

    pub(super) fn collection_length(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let collection = self.pop()?;
        let id = self.expect_ref(collection, "length requires a collection")?;
        let len = match self.continuation.heap.get(id as usize) {
            Some(HeapCell::Array(items)) => items.len(),
            _ => {
                return Err(CoreError::TypeError(
                    "length requires a list or array".to_string(),
                ))
            }
        };
        self.continuation.stack.push(Value::Number(len as f64));
        self.advance_pc().map(|()| None)
    }

    pub(super) fn watch_iter(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let top = self
            .continuation
            .stack
            .last()
            .cloned()
            .ok_or(CoreError::StackUnderflow)?;
        let id = self.expect_ref(top, "for requires a list or array")?;
        if !matches!(
            self.continuation.heap.get(id as usize),
            Some(HeapCell::Array(_))
        ) {
            return Err(CoreError::TypeError(
                "for requires a list or array".to_string(),
            ));
        }
        self.continuation.iterating.push(id);
        self.advance_pc().map(|()| None)
    }

    pub(super) fn unwatch_iter(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        if self.continuation.iterating.pop().is_none() {
            return Err(CoreError::TypeError(
                "unwatch without an active for".to_string(),
            ));
        }
        self.advance_pc().map(|()| None)
    }
}
