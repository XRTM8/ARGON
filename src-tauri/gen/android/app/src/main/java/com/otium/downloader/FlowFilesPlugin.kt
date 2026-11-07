package com.otium.downloader

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.provider.OpenableColumns
import android.webkit.WebView
import android.webkit.MimeTypeMap
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.io.File
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Native file consent and explicit clipboard actions; no background clipboard polling. */
@TauriPlugin
class FlowFilesPlugin(private val activity: Activity) : Plugin(activity) {
    private val worker = Executors.newSingleThreadExecutor()
    private val picking = AtomicBoolean(false)
    private val staging get() = File(activity.cacheDir, "argon-share")

    override fun load(webView: WebView) {
        // This runs at process startup, before transfers exist. Never remove current-session files.
        worker.execute {
            val cutoff = System.currentTimeMillis() - 24 * 60 * 60 * 1000L
            staging.listFiles()?.filter { it.lastModified() < cutoff }?.forEach {
                if (it.canonicalFile.parentFile == staging.canonicalFile) it.deleteRecursively()
            }
        }
    }

    @Command
    fun chooseFiles(invoke: Invoke) {
        if (!picking.compareAndSet(false, true)) { invoke.reject("A file picker is already open"); return }
        try {
            val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = "*/*"
                putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
            }
            startActivityForResult(invoke, intent, "filesChosen")
        } catch (error: Exception) {
            picking.set(false)
            invoke.reject(error.message)
        }
    }

    @ActivityCallback
    fun filesChosen(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK) {
            picking.set(false)
            invoke.resolveObject(mapOf("paths" to emptyList<String>()))
            return
        }
        worker.execute {
            val created = mutableListOf<File>()
            try {
                val data = result.data
                val uris = if (data?.clipData != null) {
                    val clips = data.clipData!!
                    require(clips.itemCount <= 32) { "Choose at most 32 files" }
                    (0 until clips.itemCount).map { clips.getItemAt(it).uri }
                } else listOfNotNull(data?.data)
                val paths = uris.map { uri ->
                    var name = "shared-file"
                    activity.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
                        if (it.moveToFirst()) name = it.getString(0) ?: name
                    }
                    name = name.replace(Regex("[\\\\/:*?\"<>|\\p{Cntrl}]"), "_").take(180).trim().trim('.')
                    if (name.isBlank()) name = "shared-file"
                    val directory = File(staging, UUID.randomUUID().toString())
                    check(directory.mkdirs()) { "Could not prepare the selected file" }
                    created.add(directory)
                    val target = File(directory, name)
                    check(target.canonicalFile.parentFile == directory.canonicalFile)
                    activity.contentResolver.openInputStream(uri).use { source ->
                        requireNotNull(source) { "Cannot read this file" }
                        target.outputStream().use { destination ->
                            val buffer = ByteArray(256 * 1024)
                            var total = 0L
                            while (true) {
                                val count = source.read(buffer)
                                if (count < 0) break
                                total += count
                                require(total <= 50L * 1024 * 1024 * 1024) { "File exceeds 50 GB" }
                                check(directory.usableSpace > count + 4L * 1024 * 1024) { "Not enough space to prepare this file" }
                                destination.write(buffer, 0, count)
                            }
                        }
                    }
                    target.absolutePath
                }
                invoke.resolveObject(mapOf("paths" to paths))
            } catch (error: Exception) {
                created.forEach { if (it.canonicalFile.parentFile == staging.canonicalFile) it.deleteRecursively() }
                invoke.reject(error.message)
            } finally { picking.set(false) }
        }
    }

    @Command
    fun openFile(invoke: Invoke) {
        try {
            val file = File(invoke.getArgs().getString("path") ?: "").canonicalFile
            val roots = listOfNotNull(activity.filesDir, activity.cacheDir, activity.getExternalFilesDir(null)).map { it.canonicalPath + File.separator }
            require(file.isFile && roots.any { file.path.startsWith(it) }) { "File is outside ARGON storage" }
            val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
            val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "application/octet-stream"
            val intent = Intent(Intent.ACTION_VIEW).apply { setDataAndType(uri, mime); addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION) }
            activity.startActivity(Intent.createChooser(intent, "ARGON"))
            invoke.resolve()
        } catch (error: Exception) { invoke.reject(error.message) }
    }

    @Command
    fun readClipboard(invoke: Invoke) {
        try {
            val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            val text = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(activity)?.toString() ?: ""
            require(text.toByteArray(Charsets.UTF_8).size <= 16 * 1024) { "Clipboard text exceeds 16 KB" }
            invoke.resolveObject(mapOf("text" to text))
        } catch (error: Exception) { invoke.reject(error.message) }
    }

    @Command
    fun writeClipboard(invoke: Invoke) {
        try {
            val text = invoke.getArgs().getString("text") ?: ""
            require(text.toByteArray(Charsets.UTF_8).size <= 16 * 1024) { "Clipboard text exceeds 16 KB" }
            val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("ARGON", text))
            invoke.resolve()
        } catch (error: Exception) { invoke.reject(error.message) }
    }
}
