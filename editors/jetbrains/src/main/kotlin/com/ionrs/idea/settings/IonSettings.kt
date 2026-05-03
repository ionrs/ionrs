package com.ionrs.idea.settings

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.PersistentStateComponent
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.State
import com.intellij.openapi.components.Storage
import com.intellij.util.xmlb.XmlSerializerUtil

@State(
    name = "IonSettings",
    storages = [Storage("ionrs.xml")],
)
@Service(Service.Level.APP)
class IonSettings : PersistentStateComponent<IonSettings> {
    var lspPath: String = "ion-lsp"
    var lspEnabled: Boolean = true

    override fun getState(): IonSettings = this

    override fun loadState(state: IonSettings) {
        XmlSerializerUtil.copyBean(state, this)
    }

    companion object {
        val instance: IonSettings
            get() = ApplicationManager.getApplication().getService(IonSettings::class.java)
    }
}
