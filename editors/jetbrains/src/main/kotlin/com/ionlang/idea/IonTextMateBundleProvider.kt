package com.ionlang.idea

import com.intellij.ide.plugins.PluginManagerCore
import com.intellij.openapi.extensions.PluginId
import org.jetbrains.plugins.textmate.api.TextMateBundleProvider

private const val PLUGIN_ID = "com.ionlang.idea"

class IonTextMateBundleProvider : TextMateBundleProvider {
    override fun getBundles(): List<TextMateBundleProvider.PluginBundle> {
        val pluginDescriptor = PluginManagerCore.getPlugin(PluginId.getId(PLUGIN_ID))
            ?: return emptyList()

        val bundlePath = pluginDescriptor.pluginPath.resolve("ion-bundle")
        return listOf(TextMateBundleProvider.PluginBundle("Ion", bundlePath))
    }
}
