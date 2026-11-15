package com.example.app

import kotlinx.coroutines.flow.Flow

/** The largest page size. */
const val MAX_PAGE = 100

val notConst = 5

/**
 * A repository of users.
 */
class UserRepository(private val api: Api) : Repository {

    /** Loads one user. */
    suspend fun load(id: String): User {
        val text = "fun fake() {"
        return api.get(id)
    }

    override fun all(): Flow<List<User>> = api.stream()

    private inline fun <reified T> parse(raw: String): T {
        TODO()
    }

    companion object {
        fun create(): UserRepository = UserRepository(Api())
    }

    inner class Cache {
        fun clear() {}
    }
}

data class User(val id: String, val name: String)

sealed interface Result {
    data class Ok(val value: String) : Result
    object Missing : Result
}

enum class Role {
    ADMIN, USER;

    fun label(): String = name.lowercase()
}

fun String.shout(): String = uppercase()

typealias Users = List<User>

fun interface Callback {
    fun invoke(value: Int)
}

private val raw = """
fun notReal() {
"""
