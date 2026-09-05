<?php

declare(strict_types=1);

namespace Fixtures\Domain;

function make_user(int $id, string $name): User
{
    return new User($id, $name);
}
