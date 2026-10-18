package com.visualworkbench.desktop

import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.*
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean

/** Optional producer runtime, extracted only for explicit giant import/rebuild.
 * Parent directory is the already-verified physical NativeCacheAnchor. These
 * files are never placed on the app DLL search path; only the owned helper loads
 * them. Incomplete owned caches are preserved and refused, never overwritten. */
internal class DesktopVipsRuntime private constructor(val directory: Path, private val leases: List<AutoCloseable>) : AutoCloseable {
    private val closed=AtomicBoolean(false)
    override fun close(){if(closed.compareAndSet(false,true))leases.asReversed().forEach{runCatching{it.close()}}}
    companion object {
        internal val pins: Map<String,String> = linkedMapOf(
            "libaom.dll" to "aaca18312104606e0e162b346636651cc2f2853fe6ae075d457198c14fa5047e",
            "libarchive-13.dll" to "c5b15ad0b74f39e79f583dfb0b58a1a99a71d1e7639fc5a648f3974743c4b6f9",
            "libc++.dll" to "1994597b48abb9ea494f43d4e5b7ae53267a7858e25d068260864667771683dc",
            "libcairo-2.dll" to "3f07c11fe7cd180a3650d2aec5d533ced04e6e27ccdbe30bbdbbcb6be06857bb",
            "libcgif-0.dll" to "f6ffa745c3ff1e5e0b3bd7a0df8976dc532a39f385c605e5c5b37eaaab5b7763",
            "libexif-12.dll" to "a3ec1040052bf6189a0c5ae7979db80ed7a263c4129a0a5129972265a0deb952",
            "libexpat-1.dll" to "b960c03868d8f3d21c0e40a0c2d8b3704d4833bdee216fb617db720d06b315a3",
            "libffi-8.dll" to "203b0b53062221bdbcefe3d82720db52b99a38c0f87b742de46d7accebce207d",
            "libfontconfig-1.dll" to "59cd7552c3a130e872e58fcc4c762180703857b92098dfc8b760cace6d081d87",
            "libfreetype-6.dll" to "b1bc712a8099dfbf60c31b2c87ce1b83632d7bf5ba455fd490348c60625f50ec",
            "libfribidi-0.dll" to "21be07e6dbfcbaa8c6d308d554ef7536b01e3d21f0d72ba03f4de28f1df3e908",
            "libgio-2.0-0.dll" to "b9cb469131059b7bc84fadebeb43c49464dbf5e73d46c0c96ffbaa8e7d823f59",
            "libglib-2.0-0.dll" to "fa5a7a698bf7ecd6767b373467346bb574a794eb8647383df6149f9c5f39417e",
            "libgmodule-2.0-0.dll" to "066d04d888c2e453a64568ea867b20c6e1fc34b8d37282b14c909aa35941f25e",
            "libgobject-2.0-0.dll" to "d2a6a5b3fe463ecabb9ca5b62ea86d55ace0d316e334663e3ccdebbdd4c72234",
            "libharfbuzz-0.dll" to "3b57e52b04349356e6e495c4cbfd9e2a4510cc955b912d8c92a7c953e480d275",
            "libheif.dll" to "b80079ed81adc7dda7a0ef9971e2b2546b718b912f5c0410c4ca6538a2f9ffa5",
            "libhwy.dll" to "a0fb9ad8520f407f98dbdcd179265afe8509e30a7b727a79b1ac1ab9e0937b24",
            "libimagequant.dll" to "991175c70c0ce41293bb83656abf2f51db9aaf7bcb16349e1ace12b8a779ca18",
            "libjpeg-62.dll" to "e797f624df8d058b98d621750a2b8a0f78464d23c1416ece8959515ea926dae3",
            "liblcms2-2.dll" to "04a46c6cc52cab5a5ddb4d985e4cbd31283a690f6843fe1e810f77e2a2df3e7b",
            "libpango-1.0-0.dll" to "5d91c9b18832eb6669ff4384fae3aaceeee0698cbde1847f94146f3f3ab47689",
            "libpangocairo-1.0-0.dll" to "24ca2b7106abb8a8171aa1e9c73a8066d7f12368d9bf696339cd6b87bf5b412f",
            "libpangoft2-1.0-0.dll" to "000d8d57bc3513d6c241c59997f96c44f0cb4d977fd87e31a53607ded8dc3d29",
            "libpixman-1-0.dll" to "ed893588fee47353fec5cf4a1775236e1a3e2272a24a3c4e89a5c96e230f5064",
            "libpng16-16.dll" to "461610d0b8d4afe82b2372786e0a9b66bce6376e16e33b79855a7f5dab4e7477",
            "librsvg-2-2.dll" to "97e5acf33963ed2848a6ba3e7d851caee9d3c94e0abdae60d757ac23e3069f75",
            "libsharpyuv-0.dll" to "429a6c855be9004a3f9cfe3745c1a4ded9a618ffd8969eca6a44d522852c236c",
            "libtiff-6.dll" to "029256630dfa91c70459e2d6ce00f8241307f221a751a6c91420e620a50ac660",
            "libuhdr.dll" to "243e7febade1e700353854fc407020ac65008752b9bb8fcbb574ee8970a69b8c",
            "libunwind.dll" to "cada28244c3d5dc816f7c3d10667c38941d741452a8657b0f4948efee4ac345d",
            "libvips-42.dll" to "4d3a83e136eb6f63fa8f4062e8343f87bfdc65afaab2eb30175d9ac844b88555",
            "libvips-cpp-42.dll" to "85651dbfbd2b489d2a87da9778855ac7a2c7cd998be0d9e1ed6777c092e70aeb",
            "libwebp-7.dll" to "27d8dc57e63fa2ac5798628d7fa3e044cc06d7dd2c005fd8a8b719c09f1a432e",
            "libwebpdemux-2.dll" to "d16119ec43a1b428f5cd85f37fd180e1bc05069d3e42235f74a50608b35c12de",
            "libwebpmux-3.dll" to "dc4ab500a11c6f8cf9689a22f1bcfda3f5db456306e25460872ba61c2cbca6b1",
            "libxml2-16.dll" to "75bf149ed95fee1b9d8672e83bc7e515c451b65648c7d62dbdb3743e2f6cc02f",
            "libz1.dll" to "40484b21bcce8db31d247974777bdc423e445baaa63fcfa091bab07e3245b959"
        )
        private const val MAX_FILE=128L*1024*1024
        private const val MAX_TOTAL=512L*1024*1024
        private fun sha(bytes:ByteArray):String=MessageDigest.getInstance("SHA-256").digest(bytes).joinToString(""){"%02x".format(it)}
        private fun write(path:Path,bytes:ByteArray){FileChannel.open(path,StandardOpenOption.CREATE_NEW,StandardOpenOption.WRITE).use{out->val b=ByteBuffer.wrap(bytes);while(b.hasRemaining())out.write(b);out.force(true)}}
        fun prepare(physicalAnchor:Path):DesktopVipsRuntime=prepare(physicalAnchor,
            NativeResources{DesktopVipsRuntime::class.java.classLoader.getResourceAsStream(it)},WindowsNativeFileGuard())
        internal fun prepare(anchor:Path,resources:NativeResources,guard:NativeFileGuard):DesktopVipsRuntime {
            val leases=mutableListOf<AutoCloseable>()
            try {
                val physical=anchor.toAbsolutePath().normalize()
                if(physical.toRealPath()!=physical)throw NativeRuntimeFailure("The giant runtime cache anchor changed.")
                var ancestor:Path?=physical
                while(ancestor!=null){val path=ancestor;guard.check(path);leases+=guard.pinDirectory(path);ancestor=path.parent}
                val root=physical.resolve("vips-runtime")
                if(!Files.exists(root,LinkOption.NOFOLLOW_LINKS))try{Files.createDirectory(root)}catch(_:FileAlreadyExistsException){}
                guard.check(root);leases+=guard.pinDirectory(root)
                val identity=pins.entries.joinToString(""){(name,hash)->"$hash  $name\n"}.toByteArray(Charsets.US_ASCII)
                val key=sha(identity);val cache=root.resolve(key);var created=false
                if(!Files.exists(cache,LinkOption.NOFOLLOW_LINKS)){
                    Files.newDirectoryStream(root).use{stream->var count=0;for(unused in stream){if(++count>=8)throw NativeRuntimeFailure("The owned giant runtime cache is full. Review old incomplete marked caches before retrying.")}}
                    try{Files.createDirectory(cache);created=true}catch(_:FileAlreadyExistsException){}
                }
                guard.check(cache);leases+=guard.pinDirectory(cache)
                val dll=cache.resolve("dll")
                val marker=("VisualWorkbench giant decoder runtime v1\n$key\n").toByteArray(Charsets.US_ASCII)
                if(created){
                    write(cache.resolve("owner"),marker);Files.createDirectory(dll);guard.check(dll);leases+=guard.pinDirectory(dll)
                    var total=0L
                    for((name,expected)in pins){
                        val target=dll.resolve(name)
                        resources.open("win32-x86-64/vips/$name")?.use{input->
                            FileChannel.open(target,StandardOpenOption.CREATE_NEW,StandardOpenOption.WRITE).use{out->
                                val hash=MessageDigest.getInstance("SHA-256");val bytes=ByteArray(65536);var size=0L
                                while(true){val n=input.read(bytes);if(n<0)break;if(n==0)throw NativeRuntimeFailure("Giant runtime resource stalled.")
                                    size=Math.addExact(size,n.toLong());total=Math.addExact(total,n.toLong())
                                    if(size>MAX_FILE||total>MAX_TOTAL)throw NativeRuntimeFailure("Giant runtime resource exceeded its bound.")
                                    hash.update(bytes,0,n);val b=ByteBuffer.wrap(bytes,0,n);while(b.hasRemaining())out.write(b)
                                }
                                if(size==0L||hash.digest().joinToString(""){"%02x".format(it)}!=expected)throw NativeRuntimeFailure("Giant runtime resource identity differs from the approved artifact.")
                                out.force(true)
                            }
                        }?:throw NativeRuntimeFailure("The approved giant decoder runtime was not packaged. Ordinary images remain supported.")
                    }
                    write(cache.resolve("ready"),identity)
                }else{guard.check(dll);leases+=guard.pinDirectory(dll)}
                fun inventory(directory:Path,expected:Set<String>){Files.newDirectoryStream(directory).use{stream->val found=hashSetOf<String>();for(path in stream){if(found.size>=expected.size)throw NativeRuntimeFailure("Unknown giant runtime entries.");found.add(path.fileName.toString())};if(found!=expected)throw NativeRuntimeFailure("The giant runtime cache is incomplete. Existing files were preserved.")}}
                inventory(cache,setOf("owner","ready","dll"));inventory(dll,pins.keys)
                for((name,expected)in listOf("owner" to marker,"ready" to identity)){
                    val path=cache.resolve(name);guard.check(path);leases+=guard.pin(path)
                    if(!Files.newInputStream(path).use{it.readNBytes(expected.size+1)}.contentEquals(expected))throw NativeRuntimeFailure("Giant runtime ownership changed.")
                }
                var total=0L
                for((name,expected)in pins){
                    val path=dll.resolve(name);guard.check(path);leases+=guard.pin(path)
                    val size=Files.size(path);total=Math.addExact(total,size)
                    if(size !in 1..MAX_FILE||total>MAX_TOTAL)throw NativeRuntimeFailure("Giant runtime file bounds changed.")
                    val hash=MessageDigest.getInstance("SHA-256");var count=0L
                    Files.newInputStream(path).use{input->val bytes=ByteArray(65536);while(true){val n=input.read(bytes);if(n<0)break;if(n==0)throw NativeRuntimeFailure("Giant runtime read stalled.");count+=n;if(count>size)throw NativeRuntimeFailure("Giant runtime changed during verification.");hash.update(bytes,0,n)}}
                    if(count!=size||hash.digest().joinToString(""){"%02x".format(it)}!=expected)throw NativeRuntimeFailure("Giant runtime hash verification failed.")
                }
                return DesktopVipsRuntime(dll,leases.toList())
            }catch(error:Throwable){leases.asReversed().forEach{runCatching{it.close()}};throw error}
        }
    }
}
