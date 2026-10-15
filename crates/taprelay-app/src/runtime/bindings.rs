use super::*;

impl Runtime {
    pub fn bindings_changed(&mut self) {
        self.bindings_revision += 1;
        if let Err(error) = self.sync_input() {
            self.error = Some(error.to_string());
        }
    }

    pub fn set_function_enabled(&mut self, id: FunctionId, enabled: bool) -> Result<()> {
        let function = self.functions.get_mut(&id).context("Unknown function")?;
        if function.enabled == enabled {
            return Ok(());
        }
        function.enabled = enabled;
        self.bindings_changed();
        Ok(())
    }

    pub(super) fn replace_shortcut(
        &mut self,
        id: FunctionId,
        slot: usize,
        shortcut: Shortcut,
    ) -> Result<()> {
        anyhow::ensure!(slot < 2, "Shortcut slot out of range");
        anyhow::ensure!(shortcut.valid(), "Invalid shortcut");
        let mut candidate = self.functions.clone();
        let function = candidate.get_mut(&id).context("Unknown function")?;
        anyhow::ensure!(function.enabled, "Function is disabled");
        if slot == function.shortcuts.len() {
            anyhow::ensure!(
                function.shortcuts.len() < 2,
                "A function may have at most two shortcuts"
            );
            function.shortcuts.push(shortcut);
        } else {
            *function
                .shortcuts
                .get_mut(slot)
                .context("Shortcut slot out of range")? = shortcut;
        }
        anyhow::ensure!(
            taprelay_core::mapping::valid(
                &candidate,
                &self.foreground_app_rules,
                &self.custom_mappings
            ),
            "Shortcut conflicts with another function"
        );
        self.functions = candidate;
        self.bindings_changed();
        Ok(())
    }

    pub fn remove_shortcut(&mut self, id: FunctionId, slot: usize) -> Result<()> {
        let function = self.functions.get_mut(&id).context("Unknown function")?;
        anyhow::ensure!(
            slot < function.shortcuts.len(),
            "Shortcut slot out of range"
        );
        function.shortcuts.remove(slot);
        self.bindings_changed();
        Ok(())
    }

    pub fn set_foreground_app_rules(
        &mut self,
        rules: taprelay_core::foreground_app::ForegroundAppRules,
    ) -> Result<()> {
        anyhow::ensure!(
            taprelay_core::mapping::valid(&self.functions, &rules, &self.custom_mappings),
            "Invalid application group or conflicting shortcut"
        );
        self.foreground_app_rules = rules;
        self.bindings_changed();
        Ok(())
    }
}
