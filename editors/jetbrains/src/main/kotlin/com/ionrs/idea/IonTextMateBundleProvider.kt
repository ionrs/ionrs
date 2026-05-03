package com.ionrs.idea

import com.intellij.openapi.application.PathManager
import com.intellij.openapi.diagnostic.Logger
import org.jetbrains.plugins.textmate.api.TextMateBundleProvider
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption

/**
 * Registers the Ion TextMate grammar with the IDE.
 *
 * The bundle ships inside the plugin JAR but the TextMate engine walks the
 * bundle as a real filesystem directory, so we extract it to the IDE system
 * directory on first use.
 */
class IonTextMateBundleProvider : TextMateBundleProvider {

    override fun getBundles(): List<TextMateBundleProvider.PluginBundle> {
        val bundleDir = extractBundle() ?: return emptyList()
        return listOf(TextMateBundleProvider.PluginBundle("Ion", bundleDir))
    }

    private fun extractBundle(): Path? {
        val target = Path.of(PathManager.getSystemPath(), "ionrs", "ion-bundle")
        return try {
            Files.createDirectories(target.resolve("Syntaxes"))
            Files.createDirectories(target.resolve("Preferences"))
            for (resource in BUNDLE_FILES) {
                val input = javaClass.getResourceAsStream("/ion-bundle/$resource")
                if (input == null) {
                    LOG.warn("Ion TextMate bundle resource missing: $resource")
                    return null
                }
                input.use { stream ->
                    Files.copy(stream, target.resolve(resource), StandardCopyOption.REPLACE_EXISTING)
                }
            }
            target
        } catch (e: Exception) {
            LOG.warn("Failed to extract Ion TextMate bundle to $target", e)
            null
        }
    }

    companion object {
        private val LOG = Logger.getInstance(IonTextMateBundleProvider::class.java)
        private val BUNDLE_FILES = listOf(
            "info.plist",
            "Syntaxes/ion.tmLanguage.json",
            "Preferences/ion.tmPreferences",
        )
    }
}
