<?php

declare(strict_types=1);

namespace Fixtures\Domain;

interface Persistable
{
    public function persist(): bool;
}

// Tracks creation timestamps shared by entities.
trait HasTimestamps
{
    protected ?\DateTimeImmutable $createdAt = null;

    public function touch(): void
    {
        $this->createdAt = new \DateTimeImmutable();
    }
}

enum Status: string
{
    case Draft = 'draft';
    case Active = 'active';
}

abstract class Entity implements Persistable
{
    use HasTimestamps;

    public function __construct(protected readonly int $id)
    {
    }

    abstract public function persist(): bool;
}

final class User extends Entity
{
    public function __construct(
        int $id,
        public string $name,
        public Status $status = Status::Draft,
    ) {
        parent::__construct($id);
    }

    public function label(): string
    {
        return "{$this->id}: {$this->name}";
    }

    public function persist(): bool
    {
        return $this->status === Status::Active;
    }
}
