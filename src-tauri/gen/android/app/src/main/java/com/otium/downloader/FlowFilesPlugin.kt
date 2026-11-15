package com.otium.downloader

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.provider.Settings
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
    private var mainWebView: WebView? = null

    override fun load(webView: WebView) {
        mainWebView = webView
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

    private fun resolveAllowedFile(rawPath: String): File {
        val file = File(rawPath).canonicalFile
        val downloadsDir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
        val moviesDir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_MOVIES)
        val picturesDir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_PICTURES)
        val musicDir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_MUSIC)
        val roots = listOfNotNull(
            activity.filesDir,
            activity.filesDir.parentFile,
            activity.cacheDir,
            activity.getExternalFilesDir(null),
            downloadsDir,
            moviesDir,
            picturesDir,
            musicDir,
            Environment.getExternalStorageDirectory()
        ).map { it.canonicalPath + File.separator }
        require(file.isFile && roots.any { file.path.startsWith(it) }) { "File is outside ARGON storage" }
        return file
    }

    @Command
    fun publishDownload(invoke: Invoke) {
        worker.execute {
            try {
                val rawPath = invoke.getArgs().getString("path") ?: ""
                val file = resolveAllowedFile(rawPath)
                val ext = file.extension.lowercase()
                val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)
                    ?: "application/octet-stream"

                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    val resolver = activity.contentResolver
                    val (collection, relativePath) = when {
                        mime.startsWith("video/") -> Pair(
                            MediaStore.Video.Media.EXTERNAL_CONTENT_URI,
                            "${Environment.DIRECTORY_MOVIES}/ARGON"
                        )
                        mime.startsWith("image/") -> Pair(
                            MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                            "${Environment.DIRECTORY_PICTURES}/ARGON"
                        )
                        mime.startsWith("audio/") -> Pair(
                            MediaStore.Audio.Media.EXTERNAL_CONTENT_URI,
                            "${Environment.DIRECTORY_MUSIC}/ARGON"
                        )
                        else -> Pair(
                            MediaStore.Downloads.EXTERNAL_CONTENT_URI,
                            "${Environment.DIRECTORY_DOWNLOADS}/ARGON"
                        )
                    }
                    val values = ContentValues().apply {
                        put(MediaStore.MediaColumns.DISPLAY_NAME, file.name)
                        put(MediaStore.MediaColumns.MIME_TYPE, mime)
                        put(MediaStore.MediaColumns.RELATIVE_PATH, relativePath)
                        put(MediaStore.MediaColumns.IS_PENDING, 1)
                    }
                    val itemUri = resolver.insert(collection, values)
                        ?: throw IllegalStateException("Could not create MediaStore entry")
                    try {
                        resolver.openOutputStream(itemUri)?.use { out ->
                            file.inputStream().use { input -> input.copyTo(out, 256 * 1024) }
                        } ?: throw IllegalStateException("Could not write MediaStore output")
                        values.clear()
                        values.put(MediaStore.MediaColumns.IS_PENDING, 0)
                        resolver.update(itemUri, values, null, null)
                        invoke.resolveObject(mapOf("uri" to itemUri.toString(), "location" to "$relativePath/${file.name}"))
                    } catch (e: Exception) {
                        resolver.delete(itemUri, null, null)
                        throw e
                    }
                } else {
                    val pubRoot = File(
                        Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS),
                        "ARGON"
                    )
                    pubRoot.mkdirs()
                    var dest = File(pubRoot, file.name)
                    if (dest.exists()) {
                        dest = File(pubRoot, "${file.nameWithoutExtension}_${System.currentTimeMillis()}.${file.extension}")
                    }
                    file.copyTo(dest, overwrite = false)
                    MediaScannerConnection.scanFile(activity, arrayOf(dest.absolutePath), arrayOf(mime), null)
                    invoke.resolveObject(mapOf("uri" to dest.absolutePath, "location" to dest.absolutePath))
                }
            } catch (error: Exception) {
                invoke.reject(error.message ?: "publish_failed")
            }
        }
    }

    @Command
    fun openFile(invoke: Invoke) {
        activity.runOnUiThread {
            try {
                val file = resolveAllowedFile(invoke.getArgs().getString("path") ?: "")
                val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
                if (file.extension.lowercase() == "apk") {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && !activity.packageManager.canRequestPackageInstalls()) {
                        val settingsIntent = Intent(
                            Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                            Uri.parse("package:${activity.packageName}")
                        ).apply { addFlags(Intent.FLAG_ACTIVITY_NEW_TASK) }
                        activity.startActivity(settingsIntent)
                        invoke.reject("android_install_permission_required")
                    } else {
                        val installIntent = Intent(Intent.ACTION_VIEW).apply {
                            setDataAndType(uri, "application/vnd.android.package-archive")
                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                        }
                        activity.startActivity(installIntent)
                        invoke.resolve()
                    }
                } else {
                    val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "application/octet-stream"
                    val intent = Intent(Intent.ACTION_VIEW).apply {
                        setDataAndType(uri, mime)
                        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                    }
                    val chooser = Intent.createChooser(intent, file.name).apply {
                        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                    }
                    try {
                        activity.startActivity(chooser)
                    } catch (e: ActivityNotFoundException) {
                        val genericIntent = Intent(Intent.ACTION_VIEW).apply {
                            setDataAndType(uri, "*/*")
                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                        }
                        activity.startActivity(Intent.createChooser(genericIntent, file.name).apply {
                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                        })
                    }
                    invoke.resolve()
                }
            } catch (error: Exception) { invoke.reject(error.message) }
        }
    }

    @Command
    fun shareFile(invoke: Invoke) {
        activity.runOnUiThread {
            try {
                val file = resolveAllowedFile(invoke.getArgs().getString("path") ?: "")
                val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
                val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "application/octet-stream"
                val intent = Intent(Intent.ACTION_SEND).apply {
                    type = mime
                    putExtra(Intent.EXTRA_STREAM, uri)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                }
                val chooser = Intent.createChooser(intent, file.name).apply {
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                }
                activity.startActivity(chooser)
                invoke.resolve()
            } catch (error: Exception) { invoke.reject(error.message) }
        }
    }

    @Command
    fun syncTransferNotification(invoke: Invoke) {
        try {
            val args = invoke.getArgs()
            val activeCount = args.optInt("activeCount", 0)
            val title = args.optString("title", "ARGON")
            val text = args.optString("text", "")
            val progress = args.optInt("progress", 0)
            val indeterminate = args.optBoolean("indeterminate", true)
            DownloadForegroundService.syncState(activity, activeCount, title, text, progress, indeterminate)
            invoke.resolve()
        } catch (error: Exception) { invoke.reject(error.message) }
    }

    @Command
    fun readClipboard(invoke: Invoke) {
        activity.runOnUiThread {
            try {
                val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                var text = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(activity)?.toString() ?: ""
                if (text.toByteArray(Charsets.UTF_8).size > 1024 * 1024) {
                    text = text.take(1024 * 1024)
                }
                invoke.resolveObject(mapOf("text" to text))
            } catch (error: Exception) { invoke.reject(error.message) }
        }
    }

    @Command
    fun writeClipboard(invoke: Invoke) {
        activity.runOnUiThread {
            try {
                val text = invoke.getArgs().getString("text") ?: ""
                val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                clipboard.setPrimaryClip(ClipData.newPlainText("ARGON", text))
                invoke.resolve()
            } catch (error: Exception) { invoke.reject(error.message) }
        }
    }

    @Command
    fun openMediaSniffer(invoke: Invoke) {
        activity.runOnUiThread {
            try {
                val initialUrl = invoke.getArgs().optString("url", "https://google.com").ifBlank { "https://google.com" }
                val dialog = android.app.Dialog(activity, android.R.style.Theme_DeviceDefault_NoActionBar_Fullscreen)
                val density = activity.resources.displayMetrics.density
                fun dp(px: Int) = (px * density).toInt()

                val root = android.widget.LinearLayout(activity).apply {
                    orientation = android.widget.LinearLayout.VERTICAL
                    setBackgroundColor(android.graphics.Color.parseColor("#0A0C10"))
                }

                val topBar = android.widget.LinearLayout(activity).apply {
                    orientation = android.widget.LinearLayout.HORIZONTAL
                    setPadding(dp(8), dp(8), dp(8), dp(8))
                    setBackgroundColor(android.graphics.Color.parseColor("#111827"))
                    gravity = android.view.Gravity.CENTER_VERTICAL
                }

                val backBtn = android.widget.Button(activity).apply {
                    text = "◀"
                    setTextColor(android.graphics.Color.parseColor("#94A3B8"))
                    setBackgroundColor(android.graphics.Color.TRANSPARENT)
                    minWidth = dp(40)
                    setPadding(dp(4), dp(4), dp(4), dp(4))
                }

                val forwardBtn = android.widget.Button(activity).apply {
                    text = "▶"
                    setTextColor(android.graphics.Color.parseColor("#94A3B8"))
                    setBackgroundColor(android.graphics.Color.TRANSPARENT)
                    minWidth = dp(40)
                    setPadding(dp(4), dp(4), dp(4), dp(4))
                }

                val urlInput = android.widget.EditText(activity).apply {
                    setText(initialUrl)
                    setTextColor(android.graphics.Color.WHITE)
                    setHintTextColor(android.graphics.Color.parseColor("#64748B"))
                    hint = "https://..."
                    setSingleLine(true)
                    setBackgroundColor(android.graphics.Color.parseColor("#1E293B"))
                    setPadding(dp(10), dp(8), dp(10), dp(8))
                    layoutParams = android.widget.LinearLayout.LayoutParams(0, android.view.ViewGroup.LayoutParams.WRAP_CONTENT, 1.0f).apply {
                        setMargins(dp(6), 0, dp(6), 0)
                    }
                    imeOptions = android.view.inputmethod.EditorInfo.IME_ACTION_GO
                    inputType = android.text.InputType.TYPE_TEXT_VARIATION_URI
                }

                var isDesktopMode = false
                val desktopBtn = android.widget.Button(activity).apply {
                    text = "📱"
                    setTextColor(android.graphics.Color.WHITE)
                    setBackgroundColor(android.graphics.Color.TRANSPARENT)
                    minWidth = dp(40)
                    setPadding(dp(4), dp(4), dp(4), dp(4))
                }

                val closeBtn = android.widget.Button(activity).apply {
                    text = "✕"
                    setTextColor(android.graphics.Color.parseColor("#EF4444"))
                    setBackgroundColor(android.graphics.Color.TRANSPARENT)
                    minWidth = dp(40)
                    setPadding(dp(4), dp(4), dp(4), dp(4))
                }

                topBar.addView(backBtn)
                topBar.addView(forwardBtn)
                topBar.addView(urlInput)
                topBar.addView(desktopBtn)
                topBar.addView(closeBtn)

                val frame = android.widget.FrameLayout(activity).apply {
                    layoutParams = android.widget.LinearLayout.LayoutParams(
                        android.view.ViewGroup.LayoutParams.MATCH_PARENT,
                        0,
                        1.0f
                    )
                }

                val sniffWebView = android.webkit.WebView(activity)
                sniffWebView.settings.apply {
                    javaScriptEnabled = true
                    domStorageEnabled = true
                    databaseEnabled = true
                    mediaPlaybackRequiresUserGesture = false
                    useWideViewPort = true
                    loadWithOverviewMode = true
                }

                val defaultMobileUa = sniffWebView.settings.userAgentString
                val desktopUa = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36"

                desktopBtn.setOnClickListener {
                    isDesktopMode = !isDesktopMode
                    desktopBtn.text = if (isDesktopMode) "🖥" else "📱"
                    sniffWebView.settings.userAgentString = if (isDesktopMode) desktopUa else defaultMobileUa
                    sniffWebView.reload()
                }

                val detectedStreams = java.util.concurrent.CopyOnWriteArrayList<String>()
                val badge = android.widget.TextView(activity).apply {
                    text = "📥 Download Media"
                    setTextColor(android.graphics.Color.WHITE)
                    setTextSize(android.util.TypedValue.COMPLEX_UNIT_SP, 14f)
                    setBackgroundColor(android.graphics.Color.parseColor("#2563EB"))
                    setPadding(dp(16), dp(12), dp(16), dp(12))
                    visibility = android.view.View.GONE
                    layoutParams = android.widget.FrameLayout.LayoutParams(
                        android.view.ViewGroup.LayoutParams.WRAP_CONTENT,
                        android.view.ViewGroup.LayoutParams.WRAP_CONTENT
                    ).apply {
                        gravity = android.view.Gravity.BOTTOM or android.view.Gravity.CENTER_HORIZONTAL
                        setMargins(0, 0, 0, dp(24))
                    }
                }

                fun onStreamFound(url: String) {
                    if (!detectedStreams.contains(url)) {
                        detectedStreams.add(url)
                        activity.runOnUiThread {
                            badge.visibility = android.view.View.VISIBLE
                            badge.text = "📥 Download Detected Media (${detectedStreams.size})"
                        }
                    }
                }

                badge.setOnClickListener {
                    val lastStream = detectedStreams.lastOrNull()
                    if (lastStream != null) {
                        dialog.dismiss()
                        val quoted = org.json.JSONObject.quote(lastStream)
                        val js = "window.dispatchEvent(new CustomEvent('argon-share-url', { detail: $quoted }));"
                        mainWebView?.post { mainWebView?.evaluateJavascript(js, null) }
                        invoke.resolveObject(mapOf("url" to lastStream))
                    }
                }

                sniffWebView.webViewClient = object : android.webkit.WebViewClient() {
                    override fun shouldInterceptRequest(
                        view: android.webkit.WebView?,
                        request: android.webkit.WebResourceRequest?
                    ): android.webkit.WebResourceResponse? {
                        val reqUrl = request?.url?.toString() ?: ""
                        val lower = reqUrl.lowercase()
                        if (lower.contains(".m3u8") || lower.contains(".mpd") || lower.contains(".mp4") ||
                            lower.contains(".webm") || lower.contains("videoplayback") ||
                            lower.contains("/hls/") || lower.contains("/dash/") || lower.contains("mime=video")) {
                            onStreamFound(reqUrl)
                        }
                        return super.shouldInterceptRequest(view, request)
                    }

                    override fun onPageFinished(view: android.webkit.WebView?, url: String?) {
                        super.onPageFinished(view, url)
                        if (url != null && !urlInput.hasFocus()) {
                            urlInput.setText(url)
                        }
                    }
                }

                backBtn.setOnClickListener {
                    if (sniffWebView.canGoBack()) sniffWebView.goBack()
                }
                forwardBtn.setOnClickListener {
                    if (sniffWebView.canGoForward()) sniffWebView.goForward()
                }
                urlInput.setOnEditorActionListener { _, actionId, _ ->
                    if (actionId == android.view.inputmethod.EditorInfo.IME_ACTION_GO || actionId == android.view.inputmethod.EditorInfo.IME_ACTION_DONE) {
                        var target = urlInput.text.toString().trim()
                        if (!target.startsWith("http://") && !target.startsWith("https://")) {
                            target = "https://" + target
                        }
                        sniffWebView.loadUrl(target)
                        true
                    } else false
                }
                closeBtn.setOnClickListener {
                    dialog.dismiss()
                    invoke.resolveObject(mapOf("cancelled" to true))
                }

                frame.addView(sniffWebView)
                frame.addView(badge)
                root.addView(topBar)
                root.addView(frame)

                dialog.setContentView(root)
                dialog.show()

                sniffWebView.loadUrl(initialUrl)
            } catch (e: Exception) {
                invoke.reject(e.message ?: "Failed to open media sniffer")
            }
        }
    }
}
