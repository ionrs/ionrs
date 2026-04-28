package com.ionlang.idea.settings

import com.intellij.openapi.options.Configurable
import com.intellij.ui.components.JBCheckBox
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBTextField
import com.intellij.util.ui.FormBuilder
import javax.swing.JComponent
import javax.swing.JPanel

class IonConfigurable : Configurable {
    private val pathField = JBTextField()
    private val enabledBox = JBCheckBox("Enable Ion language server")

    private var panel: JPanel? = null

    override fun getDisplayName(): String = "Ion"

    override fun createComponent(): JComponent {
        val settings = IonSettings.instance
        pathField.text = settings.lspPath
        enabledBox.isSelected = settings.lspEnabled

        val builtPanel = FormBuilder.createFormBuilder()
            .addComponent(enabledBox)
            .addLabeledComponent(JBLabel("Path to ion-lsp binary:"), pathField, 1, false)
            .addComponentFillVertically(JPanel(), 0)
            .panel
        panel = builtPanel
        return builtPanel
    }

    override fun isModified(): Boolean {
        val settings = IonSettings.instance
        return pathField.text != settings.lspPath || enabledBox.isSelected != settings.lspEnabled
    }

    override fun apply() {
        val settings = IonSettings.instance
        settings.lspPath = pathField.text.trim().ifEmpty { "ion-lsp" }
        settings.lspEnabled = enabledBox.isSelected
    }

    override fun reset() {
        val settings = IonSettings.instance
        pathField.text = settings.lspPath
        enabledBox.isSelected = settings.lspEnabled
    }

    override fun disposeUIResources() {
        panel = null
    }
}
