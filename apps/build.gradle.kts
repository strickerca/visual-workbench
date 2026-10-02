import groovy.json.JsonOutput
import java.security.MessageDigest
import javax.xml.XMLConstants
import javax.xml.parsers.DocumentBuilderFactory
import org.gradle.api.artifacts.FileCollectionDependency
import org.gradle.api.artifacts.component.ModuleComponentIdentifier
import org.gradle.api.artifacts.component.ProjectComponentIdentifier
import org.gradle.api.artifacts.result.ResolvedArtifactResult
import org.gradle.api.artifacts.result.UnresolvedArtifactResult
import org.gradle.api.artifacts.result.UnresolvedComponentResult
import org.gradle.api.artifacts.result.UnresolvedDependencyResult
import org.gradle.api.attributes.Usage
import org.gradle.api.file.FileCollection
import org.gradle.api.tasks.SourceSetContainer
import org.gradle.maven.MavenModule
import org.gradle.maven.MavenPomArtifact
import org.jetbrains.kotlin.gradle.dsl.KotlinMultiplatformExtension
import org.jetbrains.kotlin.gradle.plugin.KotlinPlatformType

buildscript {
    repositories { google(); mavenCentral() }
    dependencies { classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.4.20") }
}
plugins {
    alias(libs.plugins.kotlin.multiplatform) apply false
    alias(libs.plugins.kotlin.jvm) apply false
    alias(libs.plugins.compose.compiler) apply false
    alias(libs.plugins.compose.multiplatform) apply false
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.kmp.library) apply false
}

// Resolve the real application graphs without selecting the shared project's secondary
// Android binary artifacts. Do not change consumer attributes or use lenient resolution.
// Gradle's query API is a legacy, maintenance-mode bridge pinned to Gradle 9.7.0 here.
// https://docs.gradle.org/current/javadoc/org/gradle/api/artifacts/result/ResolutionResult.html
// https://docs.gradle.org/current/javadoc/org/gradle/api/artifacts/query/ArtifactResolutionQuery.html
// https://docs.gradle.org/current/javadoc/org/gradle/api/artifacts/result/ArtifactResolutionResult.html
// https://docs.gradle.org/current/javadoc/org/gradle/api/artifacts/result/ComponentArtifactsResult.html
val applicationLicenseConfigurations = listOf(
    ":android" to "debugRuntimeClasspath",
    ":android" to "releaseRuntimeClasspath",
    ":desktop" to "runtimeClasspath",
    ":shared" to "desktopRuntimeClasspath",
    ":pen-probe" to "debugRuntimeClasspath",
    ":pen-probe" to "releaseRuntimeClasspath",
    ":video-bench" to "debugRuntimeClasspath",
    ":video-bench" to "releaseRuntimeClasspath",
)
val applicationLicenseDirectory = layout.buildDirectory.dir("reports/dependency-license")
val generateApplicationLicenseReport = tasks.register("generateApplicationLicenseReport") {
    outputs.dir(applicationLicenseDirectory)
    outputs.upToDateWhen { false }
    notCompatibleWithConfigurationCache("Resolves application graphs and queries exact Maven POM metadata")
    doLast {
        val reportDirectory = applicationLicenseDirectory.get().asFile
        val reportFile = reportDirectory.resolve("dependencies.json")
        // A failed new census must not leave a prior success report available to the gate.
        if (reportFile.exists()) check(reportFile.delete()) { "Cannot remove stale dependency census" }
        val componentIds = linkedMapOf<String, ModuleComponentIdentifier>()
        val configurationCensus = linkedMapOf<String, List<String>>()
        val firstPartyProjects = rootProject.allprojects.map { it.path }.toSet()
        val reachableProjects = applicationLicenseConfigurations.map { it.first }.toMutableSet()
        val runtimeSources = applicationLicenseConfigurations.map { (projectPath, configurationName) ->
            val source = project(projectPath).configurations.findByName(configurationName)
                ?: error("Missing required application configuration $projectPath:$configurationName")
            check(source.isCanBeResolved) { "Application configuration is not resolvable: $projectPath:$configurationName" }
            source
        }
        fun coordinate(id: ModuleComponentIdentifier) = "${id.group}:${id.module}:${id.version}"
        fun sha256(bytes: ByteArray) = MessageDigest.getInstance("SHA-256")
            .digest(bytes).joinToString("") { "%02x".format(it.toInt() and 0xff) }

        applicationLicenseConfigurations.forEachIndexed { index, (projectPath, configurationName) ->
            val source = runtimeSources[index]
            val graph = source.incoming.resolutionResult
            val failures = graph.allDependencies.filterIsInstance<UnresolvedDependencyResult>()
            check(failures.isEmpty()) {
                "Unresolved application dependencies in $projectPath:$configurationName: " +
                    failures.joinToString { "${it.attempted.displayName}: ${it.failure.message}" }
            }
            val ids = graph.allComponents.mapNotNull { component ->
                when (val id = component.id) {
                    is ModuleComponentIdentifier -> id
                    is ProjectComponentIdentifier -> {
                        check(id.build.buildPath == ":" && id.projectPath in firstPartyProjects) {
                            "Included-build project needs reviewed license metadata: ${id.displayName}"
                        }
                        reachableProjects.add(id.projectPath)
                        null // First-party external transitives remain in allComponents.
                    }
                    else -> error("Unmapped component in application graph: ${id.displayName}")
                }
            }
            check(ids.isNotEmpty()) { "Empty application dependency census: $projectPath:$configurationName" }
            ids.forEach { componentIds[coordinate(it)] = it }
            configurationCensus["$projectPath:$configurationName"] = ids.map(::coordinate).distinct().sorted()
        }

        // File dependencies do not appear in ResolutionResult. Inspect every runtime
        // declaration hierarchy of each graph-reachable first-party project, including
        // :shared's Android hierarchy (which :android's own hierarchy cannot include).
        // A configuration-only probe classified Compose's devCompileOnly files as
        // this project's Java/Kotlin main class outputs, produced by compileJava and
        // compileKotlin; none of its runtime hierarchies includes them. Compiler-only
        // outputs and AGP SDK compiler inputs are outside this shipping audit scope.
        // First-party compiler/resource outputs are identified by source-set output
        // files and their producing tasks; names such as devRuntimeOnly grant no trust.
        val localRuntimeHierarchies = linkedMapOf<String, List<String>>()
        val firstPartyRuntimeFiles = mutableListOf<Map<String, Any>>()
        val emptyRuntimeFileDeclarations = mutableListOf<Map<String, Any>>()
        reachableProjects.sorted().forEach { projectPath ->
            val owner = project(projectPath)
            val scopes = owner.configurations.filter { configuration ->
                configuration in runtimeSources ||
                    configuration.name.endsWith("RuntimeClasspath", ignoreCase = true) ||
                    configuration.name.endsWith("RuntimeElements", ignoreCase = true) ||
                    configuration.attributes.getAttribute(Usage.USAGE_ATTRIBUTE)?.name == Usage.JAVA_RUNTIME
            }
            check(scopes.isNotEmpty()) { "No runtime declaration scope for first-party project $projectPath" }
            val hierarchy = scopes.flatMap { it.hierarchy }.distinctBy { it.name }
            val mainOutput = owner.extensions.findByType(SourceSetContainer::class.java)
                ?.findByName("main")?.output
            val mainOutputCollections = mutableListOf<FileCollection>()
            val outputFiles = linkedSetOf<java.io.File>()
            mainOutput?.let { output ->
                // Additional registered output directories may contain copied vendor
                // files. Only the source set's classes and processed resources qualify.
                outputFiles.addAll((output.classesDirs.files + listOfNotNull(output.resourcesDir))
                    .map { it.canonicalFile }.toSet()
                    .intersect(output.files.map { it.canonicalFile }.toSet()))
                mainOutputCollections.add(output)
            }
            // KMP JVM targets expose compilation outputs instead of Java's "main".
            // https://kotlinlang.org/api/kotlin-gradle-plugin/kotlin-gradle-plugin-api/org.jetbrains.kotlin.gradle.plugin/-kotlin-compilation-output/
            owner.extensions.findByType(KotlinMultiplatformExtension::class.java)?.targets
                ?.filter { it.platformType == KotlinPlatformType.jvm }?.forEach { target ->
                    target.compilations.findByName("main")?.output?.let { output ->
                        outputFiles.addAll((output.classesDirs.files + output.resourcesDir)
                            .map { it.canonicalFile }.toSet()
                            .intersect(output.allOutputs.files.map { it.canonicalFile }.toSet()))
                        mainOutputCollections.add(output.allOutputs)
                    }
                }
            val outputProducers = linkedSetOf<org.gradle.api.Task>()
            val pendingProducers = ArrayDeque<org.gradle.api.Task>()
            mainOutputCollections.forEach { output ->
                output.buildDependencies.getDependencies(null).forEach { pendingProducers.add(it) }
            }
            while (pendingProducers.isNotEmpty()) {
                val task = pendingProducers.removeFirst()
                if (outputProducers.add(task)) {
                    task.taskDependencies.getDependencies(task).forEach { pendingProducers.add(it) }
                }
            }
            val declaredOutputFiles = outputProducers.filter { it.project == owner }.associateWith { task ->
                task.outputs.files.files.map { it.canonicalFile }.toSet()
            }
            val buildDirectory = owner.layout.buildDirectory.get().asFile.canonicalFile.toPath()
            hierarchy.forEach { configuration ->
                configuration.dependencies.filterIsInstance<FileCollectionDependency>().forEach dependencyLoop@ { dependency ->
                    val files = dependency.files.files.map { it.canonicalFile }.toSet()
                    val producers = dependency.files.buildDependencies.getDependencies(null)
                    if (files.isEmpty() && producers.isEmpty()) {
                        // Kotlin may register empty friend-path collections. Record the
                        // zero-artifact observation; it grants no file/license approval.
                        emptyRuntimeFileDeclarations.add(linkedMapOf(
                            "projectPath" to projectPath,
                            "configuration" to configuration.name,
                            "fileCount" to 0,
                            "producerTasks" to emptyList<String>(),
                        ))
                        return@dependencyLoop
                    }
                    // Some Kotlin-generated collections omit builtBy. Reconstruct the
                    // binding from exact declared outputs of the source-set producer tasks.
                    val bindings = files.associateWith { file -> declaredOutputFiles.filterValues { file in it }.keys }
                    val firstPartyOutput = files.isNotEmpty() && files.all {
                        it in outputFiles && it.toPath().startsWith(buildDirectory)
                    } && bindings.values.all { it.isNotEmpty() } &&
                        producers.all { it.project == owner && it in outputProducers }
                    check(firstPartyOutput) {
                        "Unbound local runtime files need reviewed license metadata: $projectPath:${configuration.name}; " +
                            "files=${files.map { it.path }.sorted()}; mainOutputs=${outputFiles.map { it.path }.sorted()}; " +
                            "fileProducers=${producers.map { it.path }.sorted()}; mainProducers=${outputProducers.map { it.path }.sorted()}"
                    }
                    firstPartyRuntimeFiles.add(linkedMapOf(
                        "projectPath" to projectPath,
                        "configuration" to configuration.name,
                        "sourceSet" to "main",
                        "outputPaths" to files.map { buildDirectory.relativize(it.toPath()).toString().replace('\\', '/') }.sorted(),
                        "producerTasks" to bindings.values.flatten().map { it.path }.distinct().sorted(),
                        "declaredProducerTasks" to producers.map { it.path }.sorted(),
                    ))
                }
            }
            localRuntimeHierarchies[projectPath] = hierarchy.map { it.name }.sorted()
        }

        val result = dependencies.createArtifactResolutionQuery()
            .forComponents(componentIds.values)
            .withArtifacts(MavenModule::class.java, MavenPomArtifact::class.java)
            .execute()
        val failures = result.components.filterIsInstance<UnresolvedComponentResult>()
        check(failures.isEmpty()) {
            "Unresolved license metadata components: " + failures.joinToString { "${it.id.displayName}: ${it.failure.message}" }
        }
        val copiedCoordinates = linkedSetOf<String>()
        val copiedPomBytes = linkedMapOf<String, ByteArray>()
        fun copyPom(key: String, bytes: ByteArray): Map<String, Any> {
            val parts = key.split(':')
            check(parts.size == 3 && parts.none { it.isBlank() }) { "Invalid exact POM coordinate: $key" }
            val relativePath = "poms/${sha256(key.toByteArray(Charsets.UTF_8))}.pom"
            val destination = reportDirectory.resolve(relativePath)
            check(destination.parentFile.mkdirs() || destination.parentFile.isDirectory) { "Cannot create POM evidence directory" }
            destination.writeBytes(bytes)
            copiedPomBytes[key] = bytes
            return linkedMapOf(
                "moduleName" to "${parts[0]}:${parts[1]}",
                "moduleVersion" to parts[2],
                "pomPath" to relativePath,
                "pomSha256" to sha256(bytes),
            )
        }
        val modules = result.resolvedComponents.map { component ->
            val id = component.id as? ModuleComponentIdentifier
                ?: error("POM query returned a non-module component: ${component.id.displayName}")
            val key = coordinate(id)
            check(key in componentIds && copiedCoordinates.add(key)) { "Unexpected or duplicate POM component: $key" }
            val artifacts = component.getArtifacts(MavenPomArtifact::class.java)
            check(artifacts.none { it is UnresolvedArtifactResult }) {
                "Unresolved POM artifact for $key: " + artifacts.filterIsInstance<UnresolvedArtifactResult>()
                    .joinToString { it.failure.message ?: it.id.displayName }
            }
            val poms = artifacts.filterIsInstance<ResolvedArtifactResult>()
            check(poms.size == 1 && artifacts.size == 1) { "Expected exactly one POM for $key; found ${artifacts.size}" }
            val bytes = poms.single().file.readBytes()
            linkedMapOf<String, Any>().apply {
                putAll(copyPom(key, bytes))
                put("configurations", configurationCensus.filterValues { key in it }.keys.sorted())
            }
        }.sortedBy { "${it["moduleName"]}:${it["moduleVersion"]}" }
        check(copiedCoordinates == componentIds.keys) { "Application graph and copied POM census do not match" }

        // Maven inherits licenses only when absent in the child. Retrieve the exact
        // declared parent chain, never infer from organization or another version.
        // https://maven.apache.org/pom.html#inheritance
        val xmlFactory = DocumentBuilderFactory.newInstance().apply {
            isNamespaceAware = true
            setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
            setFeature("http://xml.org/sax/features/external-general-entities", false)
            setFeature("http://xml.org/sax/features/external-parameter-entities", false)
            setAttribute(XMLConstants.ACCESS_EXTERNAL_DTD, "")
            setAttribute(XMLConstants.ACCESS_EXTERNAL_SCHEMA, "")
        }
        fun parentNeeded(bytes: ByteArray): String? {
            val root = xmlFactory.newDocumentBuilder().parse(java.io.ByteArrayInputStream(bytes)).documentElement
            val namespace = root.namespaceURI.orEmpty()
            check((root.localName ?: root.nodeName) == "project" &&
                namespace in setOf("", "http://maven.apache.org/POM/4.0.0")) { "Invalid Maven POM namespace" }
            fun children(element: org.w3c.dom.Element, name: String) = (0 until element.childNodes.length)
                .mapNotNull { element.childNodes.item(it) as? org.w3c.dom.Element }
                .filter { (it.localName ?: it.nodeName) == name && it.namespaceURI.orEmpty() == namespace }
            val licenses = children(root, "licenses").flatMap { children(it, "license") }
            if (licenses.isNotEmpty()) return null
            val parents = children(root, "parent")
            check(parents.size <= 1) { "Duplicate parent declarations in Maven POM" }
            val parent = parents.singleOrNull() ?: return null
            val parts = listOf("groupId", "artifactId", "version").map { tag ->
                val entries = children(parent, tag)
                check(entries.size == 1) { "Missing or duplicate parent $tag" }
                entries.single().textContent.trim().also { value ->
                    check(value.isNotBlank() && ':' !in value && value.none { it.isWhitespace() } &&
                        '$' !in value && '{' !in value && '}' !in value) { "Parent coordinate is not literal and exact" }
                }
            }
            return parts.joinToString(":")
        }
        val parentPoms = linkedMapOf<String, Map<String, Any>>()
        fun resolveParent(key: String): ByteArray {
            copiedPomBytes[key]?.let { bytes ->
                parentPoms.putIfAbsent(key, copyPom(key, bytes))
                return bytes
            }
            val parts = key.split(':')
            val query = dependencies.createArtifactResolutionQuery()
                .forModule(parts[0], parts[1], parts[2])
                .withArtifacts(MavenModule::class.java, MavenPomArtifact::class.java).execute()
            check(query.components.none { it is UnresolvedComponentResult } && query.components.size == 1 &&
                query.resolvedComponents.size == 1) { "Parent POM component could not be resolved: $key" }
            val component = query.resolvedComponents.single()
            val id = component.id as? ModuleComponentIdentifier ?: error("Parent POM is not a module: $key")
            check(coordinate(id) == key) { "Parent POM query returned different coordinates: $key" }
            val artifacts = component.getArtifacts(MavenPomArtifact::class.java)
            check(artifacts.size == 1 && artifacts.single() is ResolvedArtifactResult) { "Parent POM artifact missing or unresolved: $key" }
            val bytes = (artifacts.single() as ResolvedArtifactResult).file.readBytes()
            parentPoms[key] = copyPom(key, bytes)
            return bytes
        }
        modules.forEach { module ->
            val key = "${module["moduleName"]}:${module["moduleVersion"]}"
            val chain = mutableListOf<String>()
            val seen = mutableSetOf(key)
            var bytes = copiedPomBytes.getValue(key)
            var parent = parentNeeded(bytes)
            while (parent != null) {
                check(chain.size < 8) { "Parent POM chain exceeds depth 8: $key" }
                val parentKey = parent
                check(seen.add(parentKey)) { "Cycle in parent POM chain: $key" }
                chain.add(parentKey)
                bytes = resolveParent(parentKey)
                parent = parentNeeded(bytes)
            }
            module["parentPomChain"] = chain
        }
        val report = linkedMapOf(
            "format" to "application-pom-census-v1",
            "gradleVersion" to gradle.gradleVersion,
            "localRuntimeHierarchies" to localRuntimeHierarchies,
            "firstPartyRuntimeFiles" to firstPartyRuntimeFiles,
            "emptyRuntimeFileDeclarations" to emptyRuntimeFileDeclarations,
            "configurations" to configurationCensus,
            "dependencies" to modules,
            "parentPoms" to parentPoms.values.sortedBy { "${it["moduleName"]}:${it["moduleVersion"]}" },
        )
        reportFile.writeText(JsonOutput.prettyPrint(JsonOutput.toJson(report)) + "\n", Charsets.UTF_8)
        logger.lifecycle("Application license census: ${modules.size} exact modules across ${configurationCensus.size} runtime configurations; ${parentPoms.size} bound parent POMs")
    }
}
tasks.register<Exec>("checkDependencyLicenses") {
    dependsOn(generateApplicationLicenseReport)
    commandLine("python", rootProject.projectDir.parentFile.resolve("tools/check_gradle_licenses.py").absolutePath,
        layout.buildDirectory.file("reports/dependency-license/dependencies.json").get().asFile.absolutePath)
}
allprojects {
    dependencyLocking { lockAllConfigurations() }
}
